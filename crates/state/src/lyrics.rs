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

/// How long a rate-limited translation waits before its one retry, when the provider
/// named no wait of its own.
const TRANSLATE_RETRY_DELAY: Duration = Duration::from_secs(60);

/// The shortest and longest a rate-limited translation waits before its one retry.
const TRANSLATE_RETRY_MIN: Duration = Duration::from_secs(10);
const TRANSLATE_RETRY_MAX: Duration = Duration::from_secs(120);

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
    /// Counts every translation run. A run whose number no longer matches was
    /// superseded, and its answer is dropped without touching the slot.
    translate_generation: u64,
    /// The track and provider a translation failure was already toasted for, so a failure
    /// is reported once per track until the provider changes or another track plays.
    translate_failed_for: Option<(String, String)>,
    /// Whether translation is on, as the settings last said. Kept so a change to the
    /// translation settings alone does not look the track up again.
    translation_enabled: bool,
    /// The name of the translation provider, as the settings last said.
    translation_provider_name: String,
    /// Whether a key is saved for the translation provider, as the settings last
    /// said. A key saved while a provider waits for one starts its translation.
    translation_has_key: bool,
    /// The custom server's address and model name, as the settings last said. An edit
    /// re-translates a sheet that has none yet, without clearing what is there.
    translation_base_url: String,
    translation_model: String,
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
            let translation_provider = settings.translation_provider();
            let translation_provider_name = translation_provider.name().to_owned();
            let translation_has_key =
                music::lyrics::translate::has_translation_key(translation_provider);
            let translation_base_url = settings.translation_base_url().to_owned();
            let translation_model = settings.translation_model().to_owned();
            let lyrics_changed =
                on != this.enabled || enabled != this.enabled_providers || priority != this.priority;
            let provider_changed = translation_provider_name != this.translation_provider_name;
            let key_flipped = !this.translation_has_key && translation_has_key;
            let base_or_model_changed = translation_base_url != this.translation_base_url
                || translation_model != this.translation_model;
            let translation_changed = translation_on != this.translation_enabled
                || provider_changed
                || translation_has_key != this.translation_has_key
                || base_or_model_changed;
            if !lyrics_changed && !translation_changed {
                return;
            }
            this.translation_enabled = translation_on;
            this.translation_provider_name = translation_provider_name;
            this.translation_has_key = translation_has_key;
            this.translation_base_url = translation_base_url;
            this.translation_model = translation_model;
            if lyrics_changed {
                this.enabled = on;
                this.enabled_providers = enabled.to_vec();
                this.priority = priority;
                this.task = None;
                this.ahead = None;
                this.ahead_of = None;
                this.translate_task = None;
                this.translate_generation = this.translate_generation.wrapping_add(1);
                this.translate_failed_for = None;
                this.forget(cx);
                this.follow(cx);
                return;
            }
            // Only the translation settings changed, so the lyrics on screen stay, and
            // any translation already running is superseded: its answer belongs to
            // settings that no longer hold.
            this.translate_generation = this.translate_generation.wrapping_add(1);
            this.translate_task = None;
            if !translation_on {
                // Switching translation off only hides the translated lanes until it is
                // switched back on.
                cx.notify();
                return;
            }
            // Picking a provider whose key is not saved yet must not attempt a
            // translation: the key field is still empty and Save has not been
            // pressed, so the attempt could only warn about the missing key. The
            // attempt runs once a key is saved, or right away for a provider that
            // needs none.
            let key_ready = !translation_provider.needs_key() || translation_has_key;
            if provider_changed {
                // A new provider translates the sheets again, but only when an
                // attempt can actually run: clearing them for one that cannot
                // would throw away translations nothing replaces yet.
                if key_ready {
                    this.clear_translations(cx);
                    this.translate_failed_for = None;
                    this.maybe_translate(cx);
                }
            } else if key_flipped {
                // A key was just saved for this provider, so sheets held from
                // another one are translated again with it.
                this.clear_translations(cx);
                this.translate_failed_for = None;
                this.maybe_translate(cx);
            } else if key_ready {
                // Translation was switched on, or the custom server's address or
                // model changed: the sheet on screen is translated if it has none
                // yet, and what is already translated is left alone.
                this.maybe_translate(cx);
            }
        })
        .detach();
        let enabled = settings.read(cx).lyrics_enabled();
        let enabled_providers = settings.read(cx).lyrics_providers().to_vec();
        let priority = settings.read(cx).lyrics_provider_priority();
        let translation_enabled = settings.read(cx).lyrics_translation_enabled();
        let translation_provider = settings.read(cx).translation_provider();
        let translation_provider_name = translation_provider.name().to_owned();
        let translation_has_key =
            music::lyrics::translate::has_translation_key(translation_provider);
        let translation_base_url = settings.read(cx).translation_base_url().to_owned();
        let translation_model = settings.read(cx).translation_model().to_owned();
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
            http: music::lyrics::translate::translation_client(),
            task: None,
            ahead: None,
            ahead_of: None,
            save: None,
            translate_task: None,
            translate_generation: 0,
            translate_failed_for: None,
            translation_enabled,
            translation_provider_name,
            translation_has_key,
            translation_base_url,
            translation_model,
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
        self.translate_generation = self.translate_generation.wrapping_add(1);
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
            // A lookup started ahead for this track must not show its answer after
            // the remembered one is up and translate the sheet a second time.
            self.ahead = None;
            self.ahead_of = None;
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
        self.translate_generation = self.translate_generation.wrapping_add(1);
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
    /// the sheet, so the track is not translated again; a failed one is neither kept nor,
    /// unless the request never completed, tried again, and a failure is toasted about
    /// once per track, naming the key only when the failure is about the key. A run the
    /// provider rate-limits waits out the asked-for wait and tries once more, and only
    /// for the same track, sheet, and settings it started with.
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
        // A provider whose key is not saved yet is never attempted: the attempt could
        // only warn about the missing key, and the settings observer starts it once
        // the key is saved.
        if provider.needs_key() && !music::lyrics::translate::has_translation_key(provider) {
            return;
        }
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
        let io = self.io.clone();
        let http = self.http.clone();
        let lyrics = hit.lyrics.clone();
        let first_request = request.clone();
        let first_lyrics = lyrics.clone();
        let outcome_request = request;
        self.translate_generation = self.translate_generation.wrapping_add(1);
        let generation = self.translate_generation;
        self.translate_task = Some(cx.spawn(async move |this, cx| {
            // The request runs on the tokio runtime, the way the lyrics lookups
            // run theirs: hyper's DNS lookup panics with "there is no reactor
            // running" when polled on GPUI's executor, and that panic aborts the
            // whole process. A failure here only ever shows the original lyrics.
            let outcome = join(io.spawn(async move {
                let outcome = music::lyrics::translate::translate_sheet(
                    &first_lyrics,
                    &first_request,
                    &http,
                )
                .await;
                match outcome {
                    Ok(translated) => Ok(translated),
                    Err(error) => {
                        // A request that never completed is worth one more try, on a
                        // fresh connection: the endpoint may only have been
                        // unreachable for a moment. A rejected request or an
                        // unreadable answer would fail again, so those are not retried.
                        if music::lyrics::translate::classify_error(&error, &first_request)
                            .is_retryable()
                        {
                            let http = music::lyrics::translate::translation_client();
                            music::lyrics::translate::translate_sheet(
                                &first_lyrics,
                                &first_request,
                                &http,
                            )
                            .await
                        } else {
                            Err(error)
                        }
                    }
                }
            }))
            .await;
            let retry = this.update(cx, |this, cx| {
                this.finish_translation_attempt(
                    outcome,
                    &id,
                    &hit,
                    &lyrics,
                    generation,
                    &outcome_request,
                    &provider_name,
                    needs_key,
                    toast_key,
                    cx,
                )
            });
            let Ok(Some(wait)) = retry else {
                return;
            };
            // The wait runs on this outer task, so the sheet is re-checked on the
            // entity's thread before the second request is spent.
            cx.background_executor().timer(wait).await;
            let attempt = this.update(cx, |this, cx| {
                this.wake_delayed_retry(generation, &id, &lyrics, provider, cx)
            });
            let Ok(Some(request)) = attempt else {
                return;
            };
            let kind_request = request.clone();
            let retry_lyrics = lyrics.clone();
            let outcome = join(io.spawn(async move {
                let http = music::lyrics::translate::translation_client();
                music::lyrics::translate::translate_sheet(&retry_lyrics, &request, &http).await
            }))
            .await;
            this.update(cx, |this, cx| {
                this.finish_delayed_attempt(
                    outcome,
                    &id,
                    &hit,
                    &lyrics,
                    generation,
                    &kind_request,
                    &provider_name,
                    needs_key,
                    toast_key,
                    cx,
                )
            })
            .ok();
        }));
    }

    /// Settles a first translation attempt on the entity's thread. A rate limit
    /// schedules the one delayed retry and answers how long to wait, with no toast
    /// yet; anything else is settled here and answers `None`. A run the world has
    /// moved past is dropped, and the sheet on screen is handed a fresh attempt.
    fn finish_translation_attempt(
        &mut self,
        outcome: anyhow::Result<music::Lyrics>,
        id: &str,
        hit: &LyricsHit,
        lyrics: &music::Lyrics,
        generation: u64,
        request: &music::lyrics::translate::TranslateRequest,
        provider_name: &str,
        needs_key: bool,
        toast_key: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> Option<Duration> {
        if let Some(reason) =
            self.translation_skip_reason(generation, id, lyrics, request.provider, cx)
        {
            if reason != "superseded" {
                self.translate_task = None;
                log::debug!("lyrics: discarding a translation: {reason}");
                self.maybe_translate(cx);
            }
            return None;
        }
        match outcome {
            Ok(translated) => {
                self.translate_task = None;
                self.keep_translation(id.to_owned(), hit.clone(), translated, cx);
                None
            }
            Err(error) => {
                let kind = music::lyrics::translate::classify_error(&error, request);
                if kind.is_rate_limited() {
                    let wait = music::lyrics::translate::failure_delay(&error)
                        .unwrap_or(TRANSLATE_RETRY_DELAY)
                        .clamp(TRANSLATE_RETRY_MIN, TRANSLATE_RETRY_MAX);
                    log::debug!("lyrics: rate-limited; retrying once in {}s", wait.as_secs());
                    return Some(wait);
                }
                self.translate_task = None;
                // Only the failure's kind is logged: a provider error can name
                // the request it failed on, and a key must never reach the log.
                log::warn!(
                    "lyrics: translation with {provider_name} failed: {}",
                    kind.token()
                );
                self.toast_translation_failure(id, provider_name, needs_key, toast_key, kind, cx);
                None
            }
        }
    }

    /// Wakes a delayed retry: re-checks the track, sheet, and settings before the
    /// second request is spent, and rebuilds the request from the settings now
    /// held. Answers `None` when the retry is skipped, with the slot settled and
    /// the sheet on screen handed a fresh attempt, or superseded, with the slot
    /// left to the run that superseded it.
    fn wake_delayed_retry(
        &mut self,
        generation: u64,
        id: &str,
        lyrics: &music::Lyrics,
        provider: music::lyrics::translate::Provider,
        cx: &mut Context<Self>,
    ) -> Option<music::lyrics::translate::TranslateRequest> {
        match self.translation_skip_reason(generation, id, lyrics, provider, cx) {
            Some("superseded") => {
                log::debug!("lyrics: skipping delayed translation retry: superseded");
                None
            }
            Some(reason) => {
                self.translate_task = None;
                log::debug!("lyrics: skipping delayed translation retry: {reason}");
                self.maybe_translate(cx);
                None
            }
            None => {
                let settings = self.settings.read(cx);
                Some(music::lyrics::translate::TranslateRequest {
                    provider: settings.translation_provider(),
                    base_url: settings.translation_base_url().to_owned(),
                    model: settings.translation_model().to_owned(),
                })
            }
        }
    }

    /// Settles the delayed retry. It never schedules another: a second rate limit
    /// is toasted about like any other failure.
    fn finish_delayed_attempt(
        &mut self,
        outcome: anyhow::Result<music::Lyrics>,
        id: &str,
        hit: &LyricsHit,
        lyrics: &music::Lyrics,
        generation: u64,
        request: &music::lyrics::translate::TranslateRequest,
        provider_name: &str,
        needs_key: bool,
        toast_key: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) =
            self.translation_skip_reason(generation, id, lyrics, request.provider, cx)
        {
            if reason != "superseded" {
                self.translate_task = None;
                log::debug!("lyrics: discarding a retried translation: {reason}");
                self.maybe_translate(cx);
            }
            return;
        }
        self.translate_task = None;
        match outcome {
            Ok(translated) => {
                self.keep_translation(id.to_owned(), hit.clone(), translated, cx)
            }
            Err(error) => {
                // Only the failure's kind is logged: a provider error can name
                // the request it failed on, and a key must never reach the log.
                let kind = music::lyrics::translate::classify_error(&error, request);
                log::warn!(
                    "lyrics: translation with {provider_name} failed: {}",
                    kind.token()
                );
                self.toast_translation_failure(id, provider_name, needs_key, toast_key, kind, cx);
            }
        }
    }

    /// Why a translation run no longer speaks for the sheet on screen, if it does
    /// not: superseded by a newer run, or the track, sheet, provider, or settings
    /// moved while it ran. `None` means the run still speaks for it.
    fn translation_skip_reason(
        &self,
        generation: u64,
        id: &str,
        lyrics: &music::Lyrics,
        provider: music::lyrics::translate::Provider,
        cx: &Context<Self>,
    ) -> Option<&'static str> {
        if self.translate_generation != generation {
            return Some("superseded");
        }
        if self.following.as_deref() != Some(id) {
            return Some("track-changed");
        }
        let Some(current) = self.current() else {
            return Some("sheet-changed");
        };
        if &current.lyrics != lyrics {
            return Some("sheet-changed");
        }
        if translated(&current.lyrics) {
            return Some("already-translated");
        }
        let settings = self.settings.read(cx);
        if settings.translation_provider() != provider {
            return Some("provider-changed");
        }
        if !settings.lyrics_translation_enabled() {
            return Some("disabled");
        }
        if provider.needs_key() && !music::lyrics::translate::has_translation_key(provider) {
            return Some("no-key");
        }
        None
    }

    /// Toasts about a translation failure once per track and provider, naming the
    /// key only when the failure is about the key, the model when the provider
    /// says the model is not there, and the connection for anything else, so a
    /// network failure never looks like a wrong key.
    fn toast_translation_failure(
        &mut self,
        id: &str,
        provider_name: &str,
        needs_key: bool,
        toast_key: Option<&'static str>,
        kind: music::lyrics::translate::FailureKind,
        cx: &mut Context<Self>,
    ) {
        let toasted = self.translate_failed_for.as_ref().is_some_and(|failed| {
            failed.0 == id && failed.1 == provider_name
        });
        if toasted {
            return;
        }
        let key = if kind == music::lyrics::translate::FailureKind::Status(404) {
            Some("toast-translation-failed-model")
        } else if needs_key && kind.is_key_error() {
            toast_key
        } else {
            Some("toast-translation-failed-network")
        };
        if let Some(key) = key {
            self.translate_failed_for = Some((id.to_owned(), provider_name.to_owned()));
            Toasts::show(Outcome::Failed, key, cx);
        }
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
    fn clear_translations(&mut self, cx: &mut Context<Self>) {
        for hit in &mut self.hits {
            drop_translations(&mut hit.lyrics);
        }
        for (_, found) in &mut self.recent {
            for hit in &mut found.hits {
                drop_translations(&mut hit.lyrics);
            }
        }
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
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
