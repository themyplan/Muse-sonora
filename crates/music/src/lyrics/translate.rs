use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::{Lyrics, WritingSystem};

/// A service that can translate lyrics into Persian.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    /// The free Google Translate endpoint, which needs no key.
    #[default]
    Google,
    /// Google's Gemini API, which needs a key saved in Settings.
    Gemini,
    /// OpenAI's own chat completions endpoint, which needs a key saved in Settings.
    OpenAI,
    /// Any endpoint that speaks the OpenAI chat completions shape, with the base URL and
    /// model name saved in Settings.
    OpenAiCompatible,
}

impl Provider {
    /// The name shown in Settings and in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "Google Translate",
            Self::Gemini => "Gemini",
            Self::OpenAI => "OpenAI",
            Self::OpenAiCompatible => "OpenAI-compatible",
        }
    }

    /// Whether the provider needs an API key saved in Settings before it can translate.
    pub fn needs_key(self) -> bool {
        !matches!(self, Self::Google)
    }
}

/// What a translation run needs: which provider to ask, and the base URL and model name
/// from Settings. Empty strings mean the provider's own default, except for
/// [`Provider::OpenAiCompatible`], which needs both spelled out.
#[derive(Clone, Debug, Default)]
pub struct TranslateRequest {
    pub provider: Provider,
    pub base_url: String,
    pub model: String,
}

/// Why a translation run failed, classified so it is safe to log and to act on: a kind
/// never carries a URL, a key, or lyrics text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The request never reached the provider: DNS or TCP connect failed.
    Connect,
    /// The request timed out.
    Timeout,
    /// The provider answered with an HTTP error status.
    Status(u16),
    /// The response body could not be read.
    Decode,
    /// The provider needs a key and none is saved.
    NoKey,
    /// The OpenAI-compatible provider needs a base URL in Settings.
    NoBaseUrl,
    /// The OpenAI-compatible provider needs a model name in Settings.
    NoModel,
    /// The AI provider answered in a shape the code does not understand.
    UnexpectedShape,
    /// The Google endpoint answered, but no chunk could be read back.
    GoogleUnparseable,
    /// The translation task was cancelled or panicked.
    Cancelled,
    /// The failure could not be classified.
    Unknown,
}

impl FailureKind {
    /// The one token written to the log for this kind.
    pub fn token(self) -> String {
        match self {
            Self::Connect => "connect".to_owned(),
            Self::Timeout => "timeout".to_owned(),
            Self::Status(status) => format!("http-{status}"),
            Self::Decode => "decode".to_owned(),
            Self::NoKey => "no-key".to_owned(),
            Self::NoBaseUrl => "no-base-url".to_owned(),
            Self::NoModel => "no-model".to_owned(),
            Self::UnexpectedShape => "unexpected-shape".to_owned(),
            Self::GoogleUnparseable => "google-unparseable".to_owned(),
            Self::Cancelled => "cancelled".to_owned(),
            Self::Unknown => "unknown".to_owned(),
        }
    }

    /// Whether the failure is about the key: no key saved, or the provider rejected
    /// the request as unauthorized. Only these deserve the "check the key" message.
    pub fn is_key_error(self) -> bool {
        matches!(self, Self::NoKey | Self::Status(401) | Self::Status(403))
    }

    /// Whether the run is worth trying once more: only failures where the request
    /// never completed. A rejected request or an unreadable answer would fail again.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Connect | Self::Timeout)
    }

    /// Classifies an error by what it carries: a kind attached by the translation
    /// code itself, a cancelled task, or a request error, whose flags and status code
    /// alone are read. The error's message is never read: it can name the request
    /// URL, which can carry the key. `None` means the error carries none of these,
    /// and the request itself must say why it failed.
    fn from_error(error: &anyhow::Error) -> Option<Self> {
        if let Some(kinded) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<KindedError>())
        {
            return Some(kinded.0);
        }
        if error
            .chain()
            .any(|cause| cause.downcast_ref::<tokio::task::JoinError>().is_some())
        {
            return Some(Self::Cancelled);
        }
        let request = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<reqwest::Error>())?;
        Some(if let Some(status) = request.status() {
            Self::Status(status.as_u16())
        } else if request.is_timeout() {
            Self::Timeout
        } else if request.is_connect() {
            Self::Connect
        } else if request.is_decode() {
            Self::Decode
        } else {
            Self::Unknown
        })
    }
}

/// An error that carries only a [`FailureKind`], so the reason a run failed survives
/// to the caller without ever naming a URL, a key, or lyrics text.
#[derive(Debug)]
pub struct KindedError(FailureKind);

impl std::fmt::Display for KindedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "translation failed: {}", self.0.token())
    }
}

impl std::error::Error for KindedError {}

/// Classifies why a translation run failed, for the log line, the message shown, and
/// whether to try again. The error itself is asked first; when it carries no request
/// at all, the request says why, because a missing key, base URL, or model fails
/// before any HTTP, and anything else that far along is an answer the code could
/// not read.
pub fn classify_error(error: &anyhow::Error, request: &TranslateRequest) -> FailureKind {
    if let Some(kind) = FailureKind::from_error(error) {
        return kind;
    }
    if request.provider == Provider::OpenAiCompatible {
        if request.base_url.trim().is_empty() {
            return FailureKind::NoBaseUrl;
        }
        if request.model.trim().is_empty() {
            return FailureKind::NoModel;
        }
    }
    if request.provider.needs_key() && !has_translation_key(request.provider) {
        return FailureKind::NoKey;
    }
    match request.provider {
        Provider::Google => FailureKind::GoogleUnparseable,
        _ => FailureKind::UnexpectedShape,
    }
}

/// A client for one translation run: fresh, so no pooled connection from an earlier
/// run is reused, with bounded waits, so a failure surfaces in seconds instead of
/// hanging on the operating system's own timeouts.
pub fn translation_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(0)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// The language every translation targets.
const TARGET: &str = "fa";

/// The most characters of joined lines one Google request carries. Longer songs split at
/// line boundaries, so no line is ever cut in half.
const GOOGLE_CHUNK: usize = 4000;

/// The free Google Translate endpoint, which needs no key.
const GOOGLE_URL: &str = "https://translate.googleapis.com/translate_a/single";

/// The Gemini API root, with the model name appended per request.
const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// OpenAI's own API root, used when Settings names no other base URL.
const OPENAI_BASE: &str = "https://api.openai.com/v1";

/// The Gemini model used when Settings names none.
const GEMINI_MODEL: &str = "gemini-2.5-flash";

/// The OpenAI model used when Settings names none.
const OPENAI_MODEL: &str = "gpt-4o-mini";

/// The instruction every AI provider gets, asking for a translation line by line.
const INSTRUCTION: &str = "Translate each line of the following song lyrics to Persian, one line at a time. Keep the same number of lines and the same order, with exactly one translation per line. Do not add explanations, numbering, or extra lines. Leave lines that contain only symbols unchanged.";

/// Translates a lyrics sheet into Persian, filling each synced line's `translated` field
/// or, for plain lyrics, the sheet's own `translated` text, with exactly one line per line
/// of the original and blanks where a line needs no translation. Secondary lanes are left
/// alone. The sheet comes back unchanged, with no network call, when no line can be
/// translated or when the lyrics are already in the Arabic script. A run that translates
/// nothing also leaves the sheet unchanged, rather than wiping what was there.
pub async fn translate_sheet(
    lyrics: &Lyrics,
    request: &TranslateRequest,
    http: &reqwest::Client,
) -> anyhow::Result<Lyrics> {
    match lyrics {
        Lyrics::Synced { lines } => {
            let texts: Vec<String> = lines.iter().map(|line| line.text.clone()).collect();
            if !translatable_sheet(&texts) {
                return Ok(lyrics.clone());
            }
            let translated = translate_lines(&texts, request, http).await?;
            if translated.iter().all(Option::is_none) {
                return Ok(lyrics.clone());
            }
            let mut lines = lines.to_vec();
            for (line, translated) in lines.iter_mut().zip(translated) {
                line.translated = translated;
            }
            Ok(Lyrics::Synced { lines: lines.into() })
        }
        Lyrics::Plain {
            text,
            romanized,
            translated: _,
        } => {
            let texts: Vec<String> = text.lines().map(str::to_owned).collect();
            if !translatable_sheet(&texts) {
                return Ok(lyrics.clone());
            }
            let translated = translate_lines(&texts, request, http).await?;
            if translated.iter().all(Option::is_none) {
                return Ok(lyrics.clone());
            }
            let translated = translated
                .into_iter()
                .map(|line| line.unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Lyrics::Plain {
                text: text.clone(),
                romanized: romanized.clone(),
                translated: Some(translated),
            })
        }
    }
}

/// Translates `lines` into Persian, answering one entry per line in order: `None` for a
/// line with nothing to translate, `Some` for a translated one. Lines with nothing to
/// translate are never sent to the provider; only the translatable lines go, and the
/// answers are mapped back by position.
pub async fn translate_lines(
    lines: &[String],
    request: &TranslateRequest,
    http: &reqwest::Client,
) -> anyhow::Result<Vec<Option<String>>> {
    let mut translated: Vec<Option<String>> = vec![None; lines.len()];
    let wanted: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| translatable(line))
        .map(|(index, line)| (index, line.as_str()))
        .collect();
    if wanted.is_empty() {
        return Ok(translated);
    }
    let answers = match request.provider {
        Provider::Google => translate_google(&wanted, http).await?,
        Provider::Gemini => translate_gemini(&wanted, request, http).await?,
        Provider::OpenAI | Provider::OpenAiCompatible => {
            translate_openai(&wanted, request, http).await?
        }
    };
    debug_assert_eq!(answers.len(), wanted.len());
    for ((index, _), answer) in wanted.into_iter().zip(answers) {
        translated[index] = answer;
    }
    Ok(translated)
}

/// Whether a sheet is worth sending at all: at least one line holds a letter, and the
/// dominant script over all line texts is not the Arabic script, which means the lyrics
/// are already Persian or Arabic.
fn translatable_sheet(texts: &[String]) -> bool {
    texts.iter().any(|text| translatable(text))
        && super::romanize::detect(texts.iter().map(String::as_str))
            != Some(WritingSystem::Arabic)
}

/// Whether a line holds anything worth translating: at least one letter. Empty lines and
/// lines of only symbols, music notes, punctuation, or brackets are left alone.
fn translatable(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

/// The translatable lines joined with newlines, for a provider that takes the whole song
/// in one request.
fn join_lines(wanted: &[(usize, &str)]) -> String {
    wanted
        .iter()
        .map(|line| line.1)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The model name from Settings, trimmed, or the provider default when Settings names
/// none.
fn model_or<'a>(request: &'a TranslateRequest, default: &'a str) -> &'a str {
    let model = request.model.trim();
    if model.is_empty() {
        default
    } else {
        model
    }
}

/// Splits a joined answer back into `count` lines, answering `None` when the answer holds
/// a different number of lines than were sent. Blank lines become `None`.
fn align_exact(answer: &str, count: usize) -> Option<Vec<Option<String>>> {
    let lines: Vec<&str> = answer.lines().collect();
    (lines.len() == count).then(|| lines.into_iter().map(clean_line).collect())
}

/// Splits a joined answer back into `count` lines by position, dropping extra lines and
/// leaving missing ones `None`. Blank lines become `None`.
fn align_lenient(answer: &str, count: usize) -> Vec<Option<String>> {
    let lines: Vec<&str> = answer.lines().collect();
    (0..count)
        .map(|index| lines.get(index).copied().and_then(clean_line))
        .collect()
}

/// Trims a translated line, answering `None` for one that holds nothing.
fn clean_line(line: &str) -> Option<String> {
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// Collapses a one-line answer into a single line, joining any stray newlines with a
/// space, so a translated line never spills onto the next one. Blank answers become
/// `None`.
fn single_line(answer: &str) -> Option<String> {
    let line = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!line.is_empty()).then_some(line)
}

/// Translates `wanted` through the free Google endpoint, in chunks of at most
/// [`GOOGLE_CHUNK`] characters joined with newlines. A chunk whose answer cannot be split
/// back into the lines that were sent is retried line by line. A line that still fails
/// stays `None`; when every line fails, the whole run fails.
async fn translate_google(
    wanted: &[(usize, &str)],
    http: &reqwest::Client,
) -> anyhow::Result<Vec<Option<String>>> {
    let mut translated = Vec::with_capacity(wanted.len());
    let mut first_failure: Option<FailureKind> = None;
    for chunk in chunk_lines(wanted) {
        let joined = chunk.join("\n");
        match google_answer(http, &joined).await {
            Ok(body) => {
                let aligned =
                    parse_google_body(&body).and_then(|answer| align_exact(&answer, chunk.len()));
                match aligned {
                    Some(lines) => translated.extend(lines),
                    None => {
                        translate_google_line_by_line(http, &chunk, &mut translated).await;
                    }
                }
            }
            Err(error) => {
                // The endpoint itself failed, so retrying line by line would only repeat
                // the failure. The chunk stays untranslated, and the first failure's
                // kind is kept so the caller can say why the run failed.
                let kind = FailureKind::from_error(&error).unwrap_or(FailureKind::Unknown);
                log::debug!("lyrics: google chunk failed: {}", kind.token());
                if first_failure.is_none() {
                    first_failure = Some(kind);
                }
                translated.extend(std::iter::repeat_n(None, chunk.len()));
            }
        }
    }
    if !translated.is_empty() && translated.iter().all(Option::is_none) {
        let kind = first_failure.unwrap_or(FailureKind::GoogleUnparseable);
        return Err(anyhow::Error::new(KindedError(kind)));
    }
    Ok(translated)
}

/// Groups the translatable lines into chunks of at most [`GOOGLE_CHUNK`] characters when
/// joined with newlines, splitting only at line boundaries. A single line longer than the
/// limit goes on its own.
fn chunk_lines<'a>(wanted: &[(usize, &'a str)]) -> Vec<Vec<&'a str>> {
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let mut width = 0usize;
    for &(_, text) in wanted {
        let line = text.chars().count();
        let joined = width + line + usize::from(!chunk.is_empty());
        if !chunk.is_empty() && joined > GOOGLE_CHUNK {
            chunks.push(std::mem::take(&mut chunk));
            width = 0;
        }
        if !chunk.is_empty() {
            width += 1;
        }
        chunk.push(text);
        width += line;
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

/// Translates each line of a chunk on its own, after the joined answer could not be split
/// back. A line that fails stays `None` without failing the rest.
async fn translate_google_line_by_line(
    http: &reqwest::Client,
    chunk: &[&str],
    translated: &mut Vec<Option<String>>,
) {
    for &line in chunk {
        let answer = google_answer(http, line)
            .await
            .ok()
            .and_then(|body| parse_google_body(&body))
            .and_then(|answer| single_line(&answer));
        translated.push(answer);
    }
}

/// Asks the free Google endpoint to translate `text`, answering the raw response body.
async fn google_answer(http: &reqwest::Client, text: &str) -> anyhow::Result<String> {
    http.post(GOOGLE_URL)
        .query(&[
            ("client", "gtx"),
            ("sl", "auto"),
            ("tl", TARGET),
            ("hl", TARGET),
            ("dt", "t"),
            ("dt", "at"),
            ("dt", "bd"),
            ("dt", "ex"),
            ("dt", "ld"),
            ("dt", "md"),
            ("dt", "qca"),
            ("dt", "rw"),
            ("dt", "rm"),
            ("dt", "ss"),
            ("ie", "UTF-8"),
            ("oe", "UTF-8"),
        ])
        .form(&[("q", text)])
        .send()
        .await
        .context("Google Translate request failed")?
        .error_for_status()
        .context("Google Translate request failed")?
        .text()
        .await
        .context("Google Translate request failed")
}

/// Reads the translation out of a Google response body. The usual shape is an array whose
/// first element holds the translated segments, each segment's first element being its
/// translated text; the alternate shape is an object with a `sentences` array of `trans`
/// fields. Either way the pieces join in order. `None` means neither shape matched.
fn parse_google_body(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    parse_google_segments(&value).or_else(|| parse_google_sentences(&value))
}

fn parse_google_segments(value: &serde_json::Value) -> Option<String> {
    let segments = value.get(0)?.as_array()?;
    let mut translated = String::new();
    for segment in segments {
        let Some(text) = segment.get(0).and_then(serde_json::Value::as_str) else {
            continue;
        };
        translated.push_str(text);
    }
    Some(translated)
}

fn parse_google_sentences(value: &serde_json::Value) -> Option<String> {
    let sentences = value.get("sentences")?.as_array()?;
    let mut translated = String::new();
    for sentence in sentences {
        let Some(text) = sentence.get("trans").and_then(serde_json::Value::as_str) else {
            continue;
        };
        translated.push_str(text);
    }
    Some(translated)
}

/// Translates `wanted` through Gemini, in a single request carrying every line. The key
/// comes from the app's credentials storage and never appears in an error or a log line.
async fn translate_gemini(
    wanted: &[(usize, &str)],
    request: &TranslateRequest,
    http: &reqwest::Client,
) -> anyhow::Result<Vec<Option<String>>> {
    let key = load_translation_key(Provider::Gemini)
        .context("Gemini needs an API key in Settings")?;
    let model = model_or(request, GEMINI_MODEL);
    let url = format!("{GEMINI_URL}/{model}:generateContent");
    let prompt = format!("{INSTRUCTION}\n\n{}", join_lines(wanted));
    let body = http
        .post(url)
        .header("x-goog-api-key", key)
        .json(&serde_json::json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": { "temperature": 0 },
        }))
        .send()
        .await
        .context("Gemini request failed")?
        .error_for_status()
        .context("Gemini request failed")?
        .json::<serde_json::Value>()
        .await
        .context("Gemini request failed")?;
    let answer = body
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(serde_json::Value::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(serde_json::Value::as_str)
        .context("Gemini answered in an unexpected shape")?;
    Ok(align_lenient(answer, wanted.len()))
}

/// Translates `wanted` through OpenAI or an OpenAI-compatible endpoint, in a single
/// request carrying every line. An OpenAI-compatible endpoint needs both a base URL and a
/// model name in Settings; OpenAI's own endpoint falls back to its defaults. The key
/// comes from the app's credentials storage and never appears in an error or a log line.
async fn translate_openai(
    wanted: &[(usize, &str)],
    request: &TranslateRequest,
    http: &reqwest::Client,
) -> anyhow::Result<Vec<Option<String>>> {
    let provider = request.provider;
    let name = provider.name();
    let base = match provider {
        Provider::OpenAiCompatible => {
            let base = request.base_url.trim();
            if base.is_empty() {
                anyhow::bail!("OpenAI-compatible needs a base URL in Settings");
            }
            base
        }
        _ => {
            let base = request.base_url.trim();
            if base.is_empty() {
                OPENAI_BASE
            } else {
                base
            }
        }
    };
    let model = match provider {
        Provider::OpenAiCompatible => {
            let model = request.model.trim();
            if model.is_empty() {
                anyhow::bail!("OpenAI-compatible needs a model name in Settings");
            }
            model
        }
        _ => model_or(request, OPENAI_MODEL),
    };
    let key = load_translation_key(provider)
        .with_context(|| format!("{name} needs an API key in Settings"))?;
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = http
        .post(url)
        .bearer_auth(key)
        .json(&serde_json::json!({
            "model": model,
            "temperature": 0,
            "messages": [
                { "role": "system", "content": INSTRUCTION },
                { "role": "user", "content": join_lines(wanted) },
            ],
        }))
        .send()
        .await
        .with_context(|| format!("{name} request failed"))?
        .error_for_status()
        .with_context(|| format!("{name} request failed"))?
        .json::<serde_json::Value>()
        .await
        .with_context(|| format!("{name} request failed"))?;
    let answer = body
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("{name} answered in an unexpected shape"))?;
    Ok(align_lenient(answer, wanted.len()))
}

/// The file that holds every saved translation key, inside the app's credentials storage.
fn keys_path() -> PathBuf {
    crate::credentials::dir("translation").join("keys.json")
}

/// The name a provider's key goes under in the keys file. Google needs no key.
fn key_slot(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Google => None,
        Provider::Gemini => Some("gemini"),
        Provider::OpenAI => Some("openai"),
        Provider::OpenAiCompatible => Some("openai-compatible"),
    }
}

/// The keys saved so far, read from the keys file. A missing or unreadable file means no
/// keys are saved; its contents are never logged.
fn saved_keys() -> BTreeMap<String, String> {
    std::fs::read(keys_path())
        .ok()
        .and_then(|body| serde_json::from_slice::<BTreeMap<String, String>>(&body).ok())
        .unwrap_or_default()
}

/// Saves `key` for `provider` in the app's credentials storage, where only the owner can
/// read it. An empty key deletes the saved key for that provider, and when no key is left
/// the file itself is removed. Google needs no key, so saving one for it does nothing.
/// Keys never go into settings, and are never logged.
pub fn save_translation_key(provider: Provider, key: &str) -> anyhow::Result<()> {
    let Some(slot) = key_slot(provider) else {
        return Ok(());
    };
    let mut keys = saved_keys();
    let key = key.trim();
    if key.is_empty() {
        keys.remove(slot);
    } else {
        keys.insert(slot.to_owned(), key.to_owned());
    }
    let path = keys_path();
    if keys.is_empty() {
        crate::credentials::remove(&path);
        return Ok(());
    }
    let body = serde_json::to_vec(&keys).context("cannot store the translation key")?;
    crate::credentials::write(&path, &body).context("cannot store the translation key")
}

/// The key saved for `provider`, if any. A missing or unreadable keys file means no key is
/// saved. Google has no key. The key itself is never logged.
pub fn load_translation_key(provider: Provider) -> Option<String> {
    let slot = key_slot(provider)?;
    saved_keys().remove(slot).filter(|key| !key.is_empty())
}

/// Whether a non-empty key is saved for `provider`. Always false for Google.
pub fn has_translation_key(provider: Provider) -> bool {
    load_translation_key(provider).is_some_and(|key| !key.is_empty())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{Lyrics, LyricsLine, Voice};

    use super::*;

    fn line(text: &str) -> LyricsLine {
        LyricsLine {
            start: Duration::ZERO,
            end: None,
            text: text.to_owned(),
            romanized: None,
            translated: None,
            words: None,
            secondary: Vec::new(),
            voice: Voice::Lead,
        }
    }

    #[test]
    fn only_google_needs_no_key() {
        assert_eq!(Provider::Google.name(), "Google Translate");
        assert!(!Provider::Google.needs_key());
        assert_eq!(Provider::Gemini.name(), "Gemini");
        assert!(Provider::Gemini.needs_key());
        assert_eq!(Provider::OpenAI.name(), "OpenAI");
        assert!(Provider::OpenAI.needs_key());
        assert_eq!(Provider::OpenAiCompatible.name(), "OpenAI-compatible");
        assert!(Provider::OpenAiCompatible.needs_key());
        assert_eq!(Provider::default(), Provider::Google);
    }

    #[test]
    fn failure_kinds_log_a_single_token() {
        assert_eq!(FailureKind::Connect.token(), "connect");
        assert_eq!(FailureKind::Timeout.token(), "timeout");
        assert_eq!(FailureKind::Status(403).token(), "http-403");
        assert_eq!(FailureKind::Decode.token(), "decode");
        assert_eq!(FailureKind::NoKey.token(), "no-key");
        assert_eq!(FailureKind::NoBaseUrl.token(), "no-base-url");
        assert_eq!(FailureKind::NoModel.token(), "no-model");
        assert_eq!(FailureKind::UnexpectedShape.token(), "unexpected-shape");
        assert_eq!(
            FailureKind::GoogleUnparseable.token(),
            "google-unparseable"
        );
        assert_eq!(FailureKind::Cancelled.token(), "cancelled");
        assert_eq!(FailureKind::Unknown.token(), "unknown");
    }

    #[test]
    fn only_key_failures_blame_the_key() {
        assert!(FailureKind::NoKey.is_key_error());
        assert!(FailureKind::Status(401).is_key_error());
        assert!(FailureKind::Status(403).is_key_error());
        assert!(!FailureKind::Status(404).is_key_error());
        assert!(!FailureKind::Status(429).is_key_error());
        assert!(!FailureKind::Connect.is_key_error());
        assert!(!FailureKind::Timeout.is_key_error());
        assert!(!FailureKind::GoogleUnparseable.is_key_error());
    }

    #[test]
    fn only_unfinished_requests_are_retried() {
        assert!(FailureKind::Connect.is_retryable());
        assert!(FailureKind::Timeout.is_retryable());
        assert!(!FailureKind::Status(403).is_retryable());
        assert!(!FailureKind::Status(429).is_retryable());
        assert!(!FailureKind::NoKey.is_retryable());
        assert!(!FailureKind::UnexpectedShape.is_retryable());
    }

    #[test]
    fn a_classified_failure_survives_the_error_chain() {
        let error = anyhow::Error::new(KindedError(FailureKind::Connect));
        assert_eq!(
            classify_error(&error, &TranslateRequest::default()),
            FailureKind::Connect
        );
    }

    #[test]
    fn parses_the_usual_google_shape() {
        let body = r#"[[["سلام","Hello",null,null,10],[" دنیا"," world",null,null,10]],null,"en"]"#;
        assert_eq!(parse_google_body(body).as_deref(), Some("سلام دنیا"));
    }

    #[test]
    fn google_segments_without_text_are_skipped() {
        let body = r#"[[["one","1",null,null,10],[null,"2",null,null,10],["three","3",null,null,10]],null,"en"]"#;
        assert_eq!(parse_google_body(body).as_deref(), Some("onethree"));
    }

    #[test]
    fn parses_the_sentences_google_shape() {
        let body = r#"{"sentences":[{"trans":"سلام","orig":"Hello"},{"trans":" دنیا","orig":" world"}]}"#;
        assert_eq!(parse_google_body(body).as_deref(), Some("سلام دنیا"));
    }

    #[test]
    fn an_unknown_google_shape_parses_to_nothing() {
        assert_eq!(parse_google_body("not json"), None);
        assert_eq!(parse_google_body("{}"), None);
        assert_eq!(parse_google_body("[]"), None);
    }

    #[test]
    fn a_joined_answer_with_the_wrong_line_count_does_not_align() {
        assert_eq!(
            align_exact("one\ntwo", 2),
            Some(vec![Some("one".to_owned()), Some("two".to_owned())])
        );
        assert_eq!(align_exact("one", 2), None);
        assert_eq!(align_exact("one\ntwo\nthree", 2), None);
        assert_eq!(
            align_exact("one\n  \nthree", 3),
            Some(vec![
                Some("one".to_owned()),
                None,
                Some("three".to_owned())
            ])
        );
    }

    #[test]
    fn a_short_ai_answer_leaves_the_missing_lines_blank() {
        assert_eq!(
            align_lenient("one", 2),
            vec![Some("one".to_owned()), None]
        );
        assert_eq!(
            align_lenient("one\ntwo\nthree", 2),
            vec![Some("one".to_owned()), Some("two".to_owned())]
        );
    }

    #[test]
    fn only_lines_with_a_letter_are_translatable() {
        assert!(translatable("Hello world"));
        assert!(translatable("Déjà vu"));
        assert!(translatable("你好"));
        assert!(translatable("[Chorus] sing"));
        assert!(!translatable(""));
        assert!(!translatable("   "));
        assert!(!translatable("♪ ♫"));
        assert!(!translatable("... !!!"));
        assert!(!translatable("[...]"));
        assert!(!translatable("123"));
    }

    #[test]
    fn long_songs_split_at_line_boundaries() {
        let wanted: Vec<(usize, &str)> = vec![(0, "aaa"), (1, "bbb"), (2, "ccc")];
        assert_eq!(chunk_lines(&wanted), vec![vec!["aaa", "bbb", "ccc"]]);

        let long = "x".repeat(GOOGLE_CHUNK);
        let wanted: Vec<(usize, &str)> = vec![(0, long.as_str()), (1, "short")];
        let chunks = chunk_lines(&wanted);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], vec![long.as_str()]);
        assert_eq!(chunks[1], vec!["short"]);
    }

    #[tokio::test]
    async fn untranslatable_lines_are_never_sent() {
        let http = reqwest::Client::new();
        let lines = vec!["♪".to_owned(), "...".to_owned()];
        let translated = translate_lines(&lines, &TranslateRequest::default(), &http)
            .await
            .expect("lines with nothing to translate need no provider");
        assert_eq!(translated, vec![None, None]);
    }

    #[tokio::test]
    async fn persian_synced_lyrics_come_back_untouched() {
        let lyrics = Lyrics::Synced {
            lines: vec![line("سلام دنیا"), line("این یک آهنگ است")].into(),
        };
        let http = reqwest::Client::new();
        let translated = translate_sheet(&lyrics, &TranslateRequest::default(), &http)
            .await
            .expect("skipped sheets do not fail");
        assert_eq!(translated, lyrics);
    }

    #[tokio::test]
    async fn persian_plain_lyrics_come_back_untouched() {
        let lyrics = Lyrics::plain("سلام دنیا\nاین یک آهنگ است");
        let http = reqwest::Client::new();
        let translated = translate_sheet(&lyrics, &TranslateRequest::default(), &http)
            .await
            .expect("skipped sheets do not fail");
        assert_eq!(translated, lyrics);
    }

    #[tokio::test]
    async fn a_sheet_with_nothing_to_translate_comes_back_untouched() {
        let lyrics = Lyrics::Synced {
            lines: vec![line("♪"), line("...")].into(),
        };
        let http = reqwest::Client::new();
        let translated = translate_sheet(&lyrics, &TranslateRequest::default(), &http)
            .await
            .expect("skipped sheets do not fail");
        assert_eq!(translated, lyrics);
    }
}
