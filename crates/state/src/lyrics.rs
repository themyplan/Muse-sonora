use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Entity, Task};
use music::lyrics::LOCAL;
use music::{LyricsHit, LyricsProvider, LyricsQuery, Track, TrackKey};
use tokio::task::JoinSet;

use crate::sheets::{Sheets, Unsaved};
use crate::{AppSettings, Io, Outcome, Playback, Queue, QueuePosition, Session, Toasts, join};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsState {
    Idle,
    Loading,
    Ready,
    Instrumental,
    Missing,
    Failed(String),
}

const SAVE_DELAY: Duration = Duration::from_millis(800);

/// How many tracks keep their answer in memory. The rest are read back from the cache database,
/// so this only has to cover the track playing, the next one and a few just played.
const RECENT: usize = 16;

pub struct Lyrics {
    state: LyricsState,
    hits: Vec<LyricsHit>,
    chosen: usize,
    picked: bool,
    settled: bool,
    revision: u64,
    following: Option<String>,
    /// The answers looked at most recently, newest first.
    recent: VecDeque<(String, Found)>,
    store: Sheets,
    /// Answers found since the last save, written together once lookups go quiet.
    unsaved: Vec<Unsaved>,
    providers: Vec<Arc<dyn LyricsProvider>>,
    /// Whether lyrics are on at all. While off, nothing is looked up and the sheet stays empty.
    enabled: bool,
    enabled_providers: Vec<String>,
    /// The order of the sources the answer on show was ranked by. A new order looks the track
    /// up again, from what is held, so the source now ranked first is the one shown.
    priority: Vec<String>,
    playback: Entity<Playback>,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
    io: Io,
    http: reqwest::Client,
    task: Option<Task<()>>,
    ahead: Option<Task<()>>,
    ahead_of: Option<String>,
    save: Option<Task<()>>,
    translate_task: Option<Task<()>>,
    /// The track and provider a translation failure was already toasted for, so a failure
    /// is reported once per track until the provider changes or another track plays.
    translate_failed_for: Option<(String, String)>,
    /// Whether translation is on, as the settings last said. Kept so a change to the
    /// translation settings alone does not look the track up again.
    translation_enabled: bool,
    /// The name of the translation provider, as the settings last said.
    translation_provider_name: String,
    /// Moves the old lyrics.json into the cache database, once.
    _migration: Task<()>,
}

impl Lyrics {
    pub fn new(
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        providers: Vec<Arc<dyn LyricsProvider>>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&playback, |this, _, cx| {
            this.follow(cx);
            this.prefetch(cx);
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.prefetch(cx)).detach();
        cx.observe(&settings, |this, settings, cx| {
            let settings = settings.read(cx);
            let on = settings.lyrics_enabled();
            let enabled = settings.lyrics_providers();
            let priority = settings.lyrics_provider_priority();
            let translation_on = settings.lyrics_translation_enabled();
            let translation_provider = settings.translation_provider().name().to_owned();
            let lyrics_changed =
                on != this.enabled || enabled != this.enabled_providers || priority != this.priority;
            let translation_changed = translation_on != this.translation_enabled
                || translation_provider != this.translation_provider_name;
            if !lyrics_changed && !translation_changed {
                return;
            }
            let provider_changed = translation_provider != this.translation_provider_name;
            this.translation_enabled = translation_on;
            this.translation_provider_name = translation_provider;
            if lyrics_changed {
                this.enabled = on;
                this.enabled_providers = enabled.to_vec();
                this.priority = priority;
                this.task = None;
                this.ahead = None;
                this.ahead_of = None;
                this.translate_task = None;
                this.translate_failed_for = None;
                this.forget(cx);
                this.follow(cx);
                return;
            }
            // Only the translation settings changed, so the lyrics on screen stay: a new
            // provider translates them again, and switching translation off only hides the
            // translated lanes until it is switched back on.
            if provider_changed {
                this.clear_translations();
                this.translate_failed_for = None;
            }
            match translation_on {
                true => this.maybe_translate(cx),
                false => cx.notify(),
            }
        })
        .detach();
        let enabled = settings.read(cx).lyrics_enabled();
        let enabled_providers = settings.read(cx).lyrics_providers().to_vec();
        let priority = settings.read(cx).lyrics_provider_priority();
        let translation_enabled = settings.read(cx).lyrics_translation_enabled();
        let translation_provider_name =
            settings.read(cx).translation_provider().name().to_owned();
        let store = Sheets::new(storage::Cache::standard());
        let moving = store.clone();
        let migration = cx
            .background_executor()
            .spawn(async move { moving.migrate() });
        Self {
            state: LyricsState::Idle,
            hits: Vec::new(),
            chosen: 0,
            picked: false,
            settled: false,
            revision: 0,
            following: None,
            recent: VecDeque::new(),
            store,
            unsaved: Vec::new(),
            providers,
            enabled,
            enabled_providers,
            priority,
            playback,
            session,
            settings,
            io,
            http: reqwest::Client::new(),
            task: None,
            ahead: None,
            ahead_of: None,
            save: None,
            translate_task: None,
            translate_failed_for: None,
            translation_enabled,
            translation_provider_name,
            _migration: migration,
        }
    }

    pub fn state(&self) -> &LyricsState {
        &self.state
    }

    pub fn following(&self) -> Option<&str> {
        self.following.as_deref()
    }

    pub fn hits(&self) -> &[LyricsHit] {
        &self.hits
    }

    pub fn current(&self) -> Option<&LyricsHit> {
        self.hits.get(self.chosen)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.hits.len() || index == self.chosen {
            return;
        }
        self.chosen = index;
        self.picked = true;
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
        self.maybe_translate(cx);
    }

    pub fn active_line(&self, cx: &Context<Self>) -> Option<usize> {
        let music::Lyrics::Synced { lines } = &self.current()?.lyrics else {
            return None;
        };
        let playback = self.playback.read(cx);
        let position = playback.shown_time(cx).0;
        music::lyrics::active(lines, position)
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        if !self.enabled {
            return self.forget(cx);
        }
        let shown = self.playback.read(cx).shown_track(cx);
        let Some(track) = shown.as_ref() else {
            return self.forget(cx);
        };
        let Some(id) = track.id.as_deref() else {
            return self.forget(cx);
        };
        if self.following.as_deref() == Some(id) {
            return;
        }
        let (track, id) = (track.clone(), id.to_owned());
        self.following = Some(id.clone());
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.translate_task = None;
        self.translate_failed_for = None;
        self.revision = self.revision.wrapping_add(1);

        let query = self.query(&track, &id, cx);
        // A file's own lyrics are never cached, so an edit to its tags shows on the next play.
        if !self.reads_file(&id, cx)
            && let Some(found) = self.recall(&id)
            && found.usable(&query, &self.enabled_providers)
            && self
                .asked(&id, cx)
                .iter()
                .all(|provider| found.covers(provider))
        {
            self.show(&found, &query, cx);
            return;
        }
        self.load(id, track, cx);
    }

    /// Puts what `found` holds from the enabled sources on screen as the settled answer.
    fn show(&mut self, found: &Found, query: &LyricsQuery, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx);
        let hits: Vec<LyricsHit> = found
            .hits
            .iter()
            .filter(|hit| settings.lyrics_provider_enabled(hit.source))
            .cloned()
            .collect();
        let priority = settings.lyrics_provider_priority();
        let instrumental = music::lyrics::instrumental(query, &hits);
        self.task = None;
        self.settled = true;
        self.hits = ordered(query, hits, &priority);
        self.state = state_for(&self.hits, instrumental);
        cx.notify();
        self.maybe_translate(cx);
        self.prefetch(cx);
    }

    /// The answer held in memory for `id`, which becomes the most recently used.
    fn recall(&mut self, id: &str) -> Option<Found> {
        let index = self.recent.iter().position(|(held, _)| held == id)?;
        let entry = self.recent.remove(index)?;
        let found = entry.1.clone();
        self.recent.push_front(entry);
        Some(found)
    }

    fn recalls(&self, id: &str) -> bool {
        self.recent.iter().any(|(held, _)| held == id)
    }

    /// Holds `found` in memory as the newest answer, letting the oldest go past `RECENT`.
    fn hold(&mut self, id: String, found: Found) {
        self.recent.retain(|(held, _)| *held != id);
        self.recent.push_front((id, found));
        self.recent.truncate(RECENT);
    }

    /// Reads the answer stored for `id` off the main thread.
    fn stored(&self, id: &str, cx: &Context<Self>) -> Task<Option<Found>> {
        let key = self.key(id, cx);
        let known: Vec<&'static str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .collect();
        let store = self.store.clone();
        cx.background_executor().spawn(async move {
            store
                .get(&key, &known)
                .map(|(hits, answered)| Found { hits, answered })
        })
    }

    /// One key per track. The stored answer covers every service that has replied, whichever
    /// are enabled now, and the selection is applied when it is shown. Services that replied
    /// empty are asked again while the stored answer holds nothing usable.
    fn key(&self, id: &str, cx: &Context<Self>) -> String {
        match self.session.read(cx).slug_for(id) {
            Some(slug) => format!("{slug}:{id}:lyrics"),
            None => format!("{id}:lyrics"),
        }
    }

    /// The providers a lookup for `id` may ask. The file's own lyrics are read only for a local
    /// file, and the services only when its metadata may go online.
    fn asked(&self, id: &str, cx: &Context<Self>) -> Vec<Arc<dyn LyricsProvider>> {
        let online = self.online(id, cx);
        let reads_file = self.reads_file(id, cx);
        let settings = self.settings.read(cx);
        self.providers
            .iter()
            .filter(|provider| settings.lyrics_provider_enabled(provider.name()))
            .filter(|provider| match provider.name() == LOCAL {
                true => reads_file,
                false => online,
            })
            .cloned()
            .collect()
    }

    fn query(&self, track: &Track, id: &str, cx: &Context<Self>) -> LyricsQuery {
        let key = self.session.read(cx).slug_for(id).map(|provider| TrackKey {
            provider,
            id: id.to_owned(),
        });
        query_for(track, key)
    }

    /// Whether the services may be asked about this track, which a local file only allows when
    /// the user lets its metadata go online.
    fn online(&self, id: &str, cx: &Context<Self>) -> bool {
        !music::is_local_id(id) || self.settings.read(cx).lyrics_for_local_files()
    }

    fn reads_file(&self, id: &str, cx: &Context<Self>) -> bool {
        music::is_local_id(id) && self.settings.read(cx).lyrics_provider_enabled(LOCAL)
    }

    /// Ends a lookup's bookkeeping; true when it was for the track on screen.
    fn finished(&mut self, id: &str) -> bool {
        let current = self.following.as_deref() == Some(id);
        if current {
            self.task = None;
        }
        if self.ahead_of.as_deref() == Some(id) {
            self.ahead_of = None;
        }
        current
    }

    fn forget(&mut self, cx: &mut Context<Self>) {
        if self.following.is_none() {
            return;
        }
        self.task = None;
        self.translate_task = None;
        self.translate_failed_for = None;
        self.following = None;
        self.hits.clear();
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.revision = self.revision.wrapping_add(1);
        self.state = LyricsState::Idle;
        cx.notify();
    }

    fn load(&mut self, id: String, track: Track, cx: &mut Context<Self>) {
        self.hits.clear();
        self.state = LyricsState::Loading;
        cx.notify();

        if self.ahead_of.as_deref() == Some(id.as_str()) && self.ahead.is_some() {
            self.task = self.ahead.take();
            self.ahead_of = None;
            return;
        }
        self.task = Some(self.fetch(id, track, cx));
    }

    fn prefetch(&mut self, cx: &mut Context<Self>) {
        if !self.enabled || self.task.is_some() {
            return;
        }
        let queue = self.playback.read(cx).shown_queue(cx);
        let Some(track) = queue.track(QueuePosition::Upcoming(0), cx) else {
            return;
        };
        let Some(id) = track.id.as_deref() else {
            return;
        };
        if self.ahead_of.as_deref() == Some(id) || self.recalls(id) {
            return;
        }
        let id = id.to_owned();
        self.ahead_of = Some(id.clone());
        self.ahead = Some(self.fetch(id, track, cx));
    }

    /// Looks a track up: in memory, then in the cache database, then with the providers. Only
    /// the services that have not answered for this track are asked, unless nothing usable is
    /// stored, and a file's own lyrics are read again on every play. What the cache holds is
    /// shown alongside the new answers.
    fn fetch(&mut self, id: String, track: Track, cx: &mut Context<Self>) -> Task<()> {
        let settings = self.settings.read(cx);
        let priority = settings.lyrics_provider_priority();
        let enabled: Vec<&'static str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .filter(|name| settings.lyrics_provider_enabled(name))
            .collect();
        let mut asked = self.asked(&id, cx);
        // Offline mode reads a file's own lyrics and what the cache kept, and asks no service.
        if crate::Network::offline(cx) {
            asked.retain(|provider| provider.name() == LOCAL);
        }
        let remembered = self.recall(&id);
        let stored = remembered.is_none().then(|| self.stored(&id, cx));
        let query = self.query(&track, &id, cx);
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let cached = match stored {
                Some(stored) => stored.await,
                None => remembered,
            };
            if let Some(found) = &cached {
                let held = found.clone();
                let id = id.clone();
                this.update(cx, |this, _| this.hold(id, held)).ok();
            }
            let cached = cached.unwrap_or_default();
            // Services that replied empty are asked again while nothing usable is stored:
            // catalogues grow, and the one source that had anything may since have been
            // switched off, leaving a stored answer with no enabled hits.
            let stored = cached.usable(&query, &enabled);
            let providers: Vec<Arc<dyn LyricsProvider>> = asked
                .into_iter()
                .filter(|provider| !stored || !cached.covers(provider))
                .collect();
            if providers.is_empty() {
                this.update(cx, |this, cx| {
                    if this.finished(&id) {
                        log::debug!("lyrics: showing the stored answer");
                        this.show(&cached, &query, cx);
                    }
                })
                .ok();
                return;
            }

            // The cached hits from enabled sources go in first, as if they had just arrived.
            let held: Vec<LyricsHit> = cached
                .hits
                .iter()
                .filter(|hit| enabled.contains(&hit.source))
                .filter(|hit| {
                    !providers
                        .iter()
                        .any(|provider| provider.name() == hit.source)
                })
                .cloned()
                .collect();
            let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel();
            if !held.is_empty() {
                sender.send((None, held)).ok();
            }
            let mut pending: Vec<&'static str> =
                providers.iter().map(|provider| provider.name()).collect();
            let ranking = query.clone();
            let worker = io.spawn(async move { gather(providers, query, sender).await });
            let mut hits = Vec::new();
            let mut displayed: Option<LyricsHit> = None;
            let mut synced = false;

            while let Some((from, mut found)) = incoming.recv().await {
                pending.retain(|name| Some(*name) != from);
                hits.append(&mut found);
                let ranked = ordered(&ranking, hits.clone(), &priority);
                let Some(best) = ranked.first().cloned() else {
                    continue;
                };
                if !synced || best.lyrics.synced() {
                    synced = best.lyrics.synced();
                    displayed = Some(best);
                }
                // Full synced lyrics can only lose to a source ranked higher, anything else to
                // whichever source answers next.
                let waiting = displayed.as_ref().is_some_and(|displayed| {
                    let place = match music::lyrics::fully_synced(&ranking, &displayed.lyrics) {
                        true => rank_in(&priority, displayed.source),
                        false => usize::MAX,
                    };
                    pending.iter().any(|name| rank_in(&priority, name) < place)
                });
                let anchor = displayed.clone();
                this.update(cx, |this, cx| {
                    if this.following.as_deref() != Some(id.as_str()) {
                        return;
                    }
                    this.paint(ranked, anchor.as_ref(), waiting, cx);
                })
                .ok();
            }

            let answered = join(worker).await;

            this.update(cx, |this, cx| {
                let current = this.finished(&id);
                match answered {
                    Ok(answered) => {
                        crate::Network::reached(cx);
                        this.keep(id, cached, &hits, answered, cx);
                        if current {
                            let instrumental = music::lyrics::instrumental(&ranking, &hits);
                            let ranked = ordered(&ranking, hits, &priority);
                            this.settle(ranked, displayed.as_ref(), instrumental, cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("lyrics: cannot look up {}: {error:#}", track.name);
                        let reason = crate::blamed(&error, cx);
                        if current {
                            this.state = LyricsState::Failed(reason);
                            cx.notify();
                        }
                    }
                }
            })
            .ok();
        })
    }

    /// Shows an answer that arrived while others are still being looked for.
    ///
    /// A synced sheet settles the question the moment it turns up, unless `waiting` says a
    /// source that could still beat it has not answered yet. Short of that, the first synced
    /// answer is put up and nothing replaces it until the question settles, and an untimed one
    /// is not shown at all while something better could still arrive.
    fn paint(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        waiting: bool,
        cx: &mut Context<Self>,
    ) {
        if self.settled {
            return;
        }
        let synced = self
            .prospect(&ranked, displayed)
            .is_some_and(|hit| hit.lyrics.synced());
        if !synced || (waiting && !self.hits.is_empty()) {
            return;
        }
        match waiting {
            false => {
                log::debug!("lyrics: settling on a synced answer as it arrives");
                self.settled = true;
            }
            true => log::debug!("lyrics: showing the first synced answer while the rest arrive"),
        }
        self.apply(ranked, displayed, cx);
    }

    fn apply(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        cx: &mut Context<Self>,
    ) {
        self.pin(ranked, displayed);
        if !self.hits.is_empty() {
            self.state = LyricsState::Ready;
        }
        cx.notify();
    }

    /// The hit a ranking would put on screen.
    fn prospect<'a>(
        &'a self,
        ranked: &'a [LyricsHit],
        displayed: Option<&'a LyricsHit>,
    ) -> Option<&'a LyricsHit> {
        let anchor = match self.picked {
            true => self.hits.get(self.chosen),
            false => displayed,
        };
        anchor
            .and_then(|anchor| ranked.iter().find(|hit| same(hit, anchor)))
            .or_else(|| ranked.first())
    }

    /// Replaces each successful source's stored answer with what it said in this lookup, including
    /// a source retried after an empty answer. Failed sources keep their previous answers.
    fn keep(
        &mut self,
        id: String,
        mut found: Found,
        hits: &[LyricsHit],
        answered: Vec<&'static str>,
        cx: &mut Context<Self>,
    ) {
        // A file's own lyrics are read again on every play, so they are never kept.
        if !self.online(&id, cx) {
            return;
        }
        let answered: Vec<&'static str> =
            answered.into_iter().filter(|name| *name != LOCAL).collect();
        if !answered.is_empty() {
            found.hits.retain(|hit| !answered.contains(&hit.source));
            found.hits.extend(
                hits.iter()
                    .filter(|hit| answered.contains(&hit.source))
                    .cloned(),
            );
            for name in answered {
                if !found.answered.contains(&name) {
                    found.answered.push(name);
                }
            }
            if !found.hits.is_empty() {
                let key = self.key(&id, cx);
                self.unsaved
                    .push(Sheets::unsaved(key, &found.hits, &found.answered));
                self.schedule_save(cx);
            }
        }
        self.hold(id, found);
    }

    /// Puts up the answer once every source has replied. Unless a word-by-word sheet already
    /// settled the question, this is the best there is and nothing may replace it afterwards.
    fn settle(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        instrumental: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.settled {
            log::debug!("lyrics: settling on the best answer");
            self.settled = true;
            self.pin(ranked, displayed);
            self.state = state_for(&self.hits, instrumental);
        }
        cx.notify();
        self.maybe_translate(cx);
        self.prefetch(cx);
    }

    fn pin(&mut self, ranked: Vec<LyricsHit>, displayed: Option<&LyricsHit>) {
        let anchor = match self.picked {
            true => self.hits.get(self.chosen).cloned(),
            false => displayed.cloned(),
        };
        let before = self.current().map(|hit| hit.lyrics.clone());
        let mut hits = ranked;
        let held = keep_displayed_first(&mut hits, anchor.as_ref());
        self.hits = hits;
        self.chosen = 0;
        self.picked &= held;
        if before.as_ref() != self.current().map(|hit| &hit.lyrics) {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Translates the lyrics on screen, once per track, when translation is on and the
    /// current sheet carries no translation yet. A translation that arrives is kept with
    /// the sheet, so the track is not translated again; a failed one is neither kept nor
    /// retried, and a provider whose key is wrong is toasted about once per track.
    fn maybe_translate(&mut self, cx: &mut Context<Self>) {
        if self.translate_task.is_some() {
            return;
        }
        let Some(id) = self.following.clone() else {
            return;
        };
        if self.state == LyricsState::Instrumental {
            return;
        }
        let Some(hit) = self.current().cloned() else {
            return;
        };
        if translated(&hit.lyrics) {
            return;
        }
        let settings = self.settings.read(cx);
        if !settings.lyrics_translation_enabled() {
            return;
        }
        let provider = settings.translation_provider();
        let provider_name = provider.name().to_owned();
        let needs_key = provider.needs_key();
        let toast_key = match &provider {
            music::lyrics::translate::Provider::Gemini => Some("toast-translation-failed-gemini"),
            music::lyrics::translate::Provider::OpenAI => Some("toast-translation-failed-openai"),
            music::lyrics::translate::Provider::OpenAiCompatible => {
                Some("toast-translation-failed-openai-compatible")
            }
            music::lyrics::translate::Provider::Google => None,
        };
        let request = music::lyrics::translate::TranslateRequest {
            provider,
            base_url: settings.translation_base_url().to_owned(),
            model: settings.translation_model().to_owned(),
        };
        let http = self.http.clone();
        let lyrics = hit.lyrics.clone();
        self.translate_task = Some(cx.spawn(async move |this, cx| {
            let outcome =
                music::lyrics::translate::translate_sheet(&lyrics, &request, &http).await;
            this.update(cx, |this, cx| {
                if this.following.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.translate_task = None;
                match outcome {
                    Ok(translated) => this.keep_translation(id, hit, translated, cx),
                    Err(_) => {
                        // The error is left out on purpose: a provider error can name the
                        // request it failed on, and a key must never reach the log.
                        log::warn!("lyrics: translation with {provider_name} failed");
                        let toasted = this.translate_failed_for.as_ref().is_some_and(|failed| {
                            failed.0 == id && failed.1 == provider_name
                        });
                        if needs_key && !toasted {
                            if let Some(key) = toast_key {
                                this.translate_failed_for = Some((id, provider_name.clone()));
                                Toasts::show(Outcome::Failed, key, cx);
                            }
                        }
                    }
                }
            })
            .ok();
        }));
    }

    /// Keeps a translated sheet where the untranslated one was: on screen, in the answers
    /// held in memory, and, for a track the services may be asked about, in the cache
    /// database, through the same unsaved path a lookup uses. A file's own lyrics are
    /// never kept, so an edit to its tags shows on the next play.
    fn keep_translation(
        &mut self,
        id: String,
        untranslated: LyricsHit,
        translated: music::Lyrics,
        cx: &mut Context<Self>,
    ) {
        let persistable = self.online(&id, cx);
        let key = persistable.then(|| self.key(&id, cx));
        let mut changed = false;
        if let Some(hit) = self.hits.iter_mut().find(|hit| same(hit, &untranslated)) {
            hit.lyrics = translated.clone();
            changed = true;
        }
        let mut pending = None;
        if let Some((_, found)) = self.recent.iter_mut().find(|(held, _)| *held == id) {
            if let Some(hit) = found.hits.iter_mut().find(|hit| same(hit, &untranslated)) {
                hit.lyrics = translated;
                changed = true;
                if let Some(key) = key {
                    pending = Some(Sheets::unsaved(key, &found.hits, &found.answered));
                }
            }
        }
        if let Some(pending) = pending {
            self.unsaved.push(pending);
            self.schedule_save(cx);
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
            cx.notify();
            // The listener may have picked another sheet while this one was translating;
            // that one still wants translating.
            if self.current().is_some_and(|hit| !same(hit, &untranslated)) {
                self.maybe_translate(cx);
            }
        }
    }

    /// Drops every translation held in memory, so the sheets are translated again with
    /// the provider now chosen. Sheets already saved in the cache database keep the old
    /// provider's translations until the track is translated again.
    fn clear_translations(&mut self) {
        for hit in &mut self.hits {
            drop_translations(&mut hit.lyrics);
        }
        for (_, found) in &mut self.recent {
            for hit in &mut found.hits {
                drop_translations(&mut hit.lyrics);
            }
        }
    }

    /// Writes the unsaved answers once lookups have gone quiet for `SAVE_DELAY`. A write that
    /// has begun always finishes, even when another answer reschedules the save.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok((store, unsaved)) = this.update(cx, |this, _| {
                (this.store.clone(), std::mem::take(&mut this.unsaved))
            }) else {
                return;
            };
            cx.background_executor()
                .spawn(async move { store.save(unsaved) })
                .detach();
        }));
    }
}

fn keep_displayed_first(hits: &mut Vec<LyricsHit>, displayed: Option<&LyricsHit>) -> bool {
    let Some(index) =
        displayed.and_then(|displayed| hits.iter().position(|hit| same(hit, displayed)))
    else {
        return false;
    };
    let displayed = hits.remove(index);
    hits.insert(0, displayed);
    true
}

fn same(left: &LyricsHit, right: &LyricsHit) -> bool {
    left.source == right.source
        && left.title == right.title
        && left.artist == right.artist
        && left.album == right.album
        && left.duration == right.duration
        && left.lyrics.worded() == right.lyrics.worded()
}

/// Whether a sheet already carries a translation: any translated line, or a translated
/// plain text.
fn translated(lyrics: &music::Lyrics) -> bool {
    match lyrics {
        music::Lyrics::Plain { translated, .. } => translated.is_some(),
        music::Lyrics::Synced { lines } => lines.iter().any(|line| line.translated.is_some()),
    }
}

/// Drops a sheet's translation, leaving the lyrics themselves alone.
fn drop_translations(lyrics: &mut music::Lyrics) {
    match lyrics {
        music::Lyrics::Plain { translated, .. } => *translated = None,
        music::Lyrics::Synced { lines } => {
            for line in Arc::make_mut(lines) {
                line.translated = None;
            }
        }
    }
}

fn ordered(query: &LyricsQuery, hits: Vec<LyricsHit>, priority: &[String]) -> Vec<LyricsHit> {
    let mut ranked = music::lyrics::rank_with_priority(query, hits, priority);
    music::lyrics::reshape(&mut ranked);
    ranked
}

/// Hits sent to a lookup, with the provider that has now answered. Hits held from an earlier
/// lookup come with no provider.
type Reply = (Option<&'static str>, Vec<LyricsHit>);

/// Where `source` stands in the user's priority. Sources missing from it come last.
fn rank_in(priority: &[String], source: &str) -> usize {
    priority
        .iter()
        .position(|name| name == source)
        .unwrap_or(priority.len())
}

/// What the services have said about one track: every hit matched to it, unranked and from every
/// source, and the services that replied. The selection is applied when it is shown.
#[derive(Clone, Default)]
struct Found {
    hits: Vec<LyricsHit>,
    answered: Vec<&'static str>,
}

impl Found {
    /// Whether an enabled source has lyrics that can be shown, or confirms an instrumental.
    fn usable(&self, query: &LyricsQuery, enabled: &[impl AsRef<str>]) -> bool {
        self.hits.iter().any(|hit| {
            enabled.iter().any(|name| name.as_ref() == hit.source)
                && (music::lyrics::eligible(query, hit)
                    || (hit.instrumental && music::lyrics::matched(query, hit)))
        })
    }

    /// Whether `provider` has replied. Callers only reuse that answer while an enabled source
    /// holds something usable. A file's own lyrics are never covered, since they are read again
    /// on every play.
    fn covers(&self, provider: &Arc<dyn LyricsProvider>) -> bool {
        provider.name() != LOCAL && self.answered.contains(&provider.name())
    }
}

fn state_for(hits: &[LyricsHit], instrumental: bool) -> LyricsState {
    match (hits.is_empty(), instrumental) {
        (false, _) => LyricsState::Ready,
        (true, true) => LyricsState::Instrumental,
        (true, false) => LyricsState::Missing,
    }
}

fn query_for(track: &Track, key: Option<TrackKey>) -> LyricsQuery {
    LyricsQuery {
        title: track.name.clone(),
        artist: track.artists.clone(),
        album: (!track.album.is_empty()).then(|| track.album.clone()),
        duration: track.duration,
        track: key,
    }
}

/// Asks every provider at once and sends each one's name and hits for the queried track as
/// they arrive. A provider that fails sends its name with no hits, so the receiver knows it is
/// done. Returns the providers that replied, leaving out any that failed.
async fn gather(
    providers: Vec<Arc<dyn LyricsProvider>>,
    query: LyricsQuery,
    sender: tokio::sync::mpsc::UnboundedSender<Reply>,
) -> anyhow::Result<Vec<&'static str>> {
    let mut tasks = JoinSet::new();
    for provider in providers {
        let query = query.clone();
        tasks.spawn(async move {
            let name = provider.name();
            let found = provider
                .search(&query)
                .await
                .inspect_err(|error| log::warn!("lyrics: {name} did not answer: {error:#}"))
                .ok()
                .map(|mut found| {
                    // Hits for another track are dropped here, before the lookup holds and
                    // copies them.
                    found.retain(|hit| music::lyrics::matched(&query, hit));
                    log::debug!(
                        "lyrics: {name} returned {} matching answer(s) for {}",
                        found.len(),
                        query.title
                    );
                    found
                });
            (name, found)
        });
    }
    let mut answered = Vec::new();
    while !tasks.is_empty() {
        tokio::select! {
            // Dropping the foreground lookup (including on a provider change) must also
            // stop its network requests. Dropping this JoinSet aborts the remaining tasks.
            _ = sender.closed() => break,
            found = tasks.join_next() => {
                if let Some(Ok((name, found))) = found {
                    if found.is_some() {
                        answered.push(name);
                    }
                    sender.send((Some(name), found.unwrap_or_default())).ok();
                }
            }
        }
    }
    Ok(answered)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use music::{Lyrics as Sheet, LyricsHit, LyricsLine, LyricsWord};

    use super::keep_displayed_first;

    fn hit(source: &'static str, lyrics: Sheet) -> LyricsHit {
        LyricsHit {
            source,
            lyrics,
            title: "title".to_owned(),
            artist: "artist".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_displayed_karaoke_result_stays_selected_after_final_ranking() {
        let plain = hit("plain", Sheet::plain("line"));
        let displayed = hit(
            "karaoke",
            Sheet::Synced {
                lines: vec![LyricsLine {
                    end: Some(Duration::from_secs(1)),
                    text: "line".to_owned(),
                    words: Some(vec![LyricsWord {
                        start: Duration::ZERO,
                        end: Duration::from_secs(1),
                        text: "line".to_owned(),
                    }]),
                    ..Default::default()
                }]
                .into(),
            },
        );
        let mut final_ranking = vec![plain, displayed.clone()];

        keep_displayed_first(&mut final_ranking, Some(&displayed));

        assert_eq!(final_ranking.first(), Some(&displayed));
    }
}
