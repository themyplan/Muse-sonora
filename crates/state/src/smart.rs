//! The four automatic playlists, each combining the streaming shelf with the
//! local one: favorites, most played, recently added and recently played.
//! They are derived data, computed on demand from the library, the history
//! and the play database, never stored as playlists of their own.

use std::collections::{HashMap, HashSet};

use gpui::{App, AppContext as _};
use music::{Playlist, Track};
use rusqlite::Connection;

use crate::{History, Library, Shelf, Sonora};

/// The liked songs, streaming and local together.
pub const SMART_FAVORITES: &str = "smart:favorites";
/// The most played songs, streaming and local together.
pub const SMART_MOST_PLAYED: &str = "smart:most-played";
/// The newest songs in the library, streaming and local together.
pub const SMART_RECENTLY_ADDED: &str = "smart:recently-added";
/// The songs played most recently, streaming and local together.
pub const SMART_RECENTLY_PLAYED: &str = "smart:recently-played";

/// The four smart ids, in the order the Playlists page lists them.
const SMART_IDS: [&str; 4] = [
    SMART_FAVORITES,
    SMART_MOST_PLAYED,
    SMART_RECENTLY_ADDED,
    SMART_RECENTLY_PLAYED,
];

/// Whether `id` names one of the automatic playlists rather than a stored one.
pub fn is_smart(id: &str) -> bool {
    id.starts_with("smart:")
}

/// The synthetic playlist for one smart id, or `None` for an unknown id.
pub fn smart_playlist(id: &str, cx: &App) -> Option<Playlist> {
    if !SMART_IDS.contains(&id) {
        return None;
    }
    let tracks = smart_tracks(id, cx);
    Some(Playlist {
        id: id.to_string(),
        name: title(id),
        owner: String::new(),
        owner_id: String::new(),
        owned: false,
        cover: tracks.first().and_then(|track| track.cover.clone()),
        track_count: tracks.len() as u32,
        modified_at: None,
        ..Default::default()
    })
}

/// All four automatic playlists, favorites first.
pub fn smart_playlists(cx: &App) -> Vec<Playlist> {
    SMART_IDS
        .iter()
        .filter_map(|id| smart_playlist(id, cx))
        .collect()
}

/// The tracks of one smart id, or an empty list for an unknown id.
pub fn smart_tracks(id: &str, cx: &App) -> Vec<Track> {
    let Some(sonora) = cx.try_global::<Sonora>() else {
        return Vec::new();
    };
    let library = sonora.library.read(cx);
    let history = sonora.history.read(cx);
    match id {
        SMART_FAVORITES => favorites(&library),
        SMART_MOST_PLAYED => most_played(&library, &history),
        SMART_RECENTLY_ADDED => recently_added(&library),
        SMART_RECENTLY_PLAYED => recently_played(&library, &history),
        _ => Vec::new(),
    }
}

/// The localized title of a smart id. Unknown ids are named as-is.
fn title(id: &str) -> String {
    let key = match id {
        SMART_FAVORITES => "smart-favorites",
        SMART_MOST_PLAYED => "smart-most-played",
        SMART_RECENTLY_ADDED => "smart-recently-added",
        SMART_RECENTLY_PLAYED => "smart-recently-played",
        _ => return id.to_string(),
    };
    i18n::lookup(key, None).to_string()
}

/// Every track the library and history can resolve, by id. History comes
/// first, then the shelves, so a library track wins over the history's copy
/// of the same song.
fn track_map(library: &Library, history: &History) -> HashMap<String, Track> {
    let mut map = HashMap::new();
    for track in history
        .tracks()
        .iter()
        .chain(library.state(Shelf::Streaming).tracks())
        .chain(library.state(Shelf::Local).tracks())
    {
        if let Some(id) = &track.id {
            map.insert(id.clone(), track.clone());
        }
    }
    map
}

/// Favorites from both shelves, newest first, with no cap.
fn favorites(library: &Library) -> Vec<Track> {
    let mut tracks: Vec<Track> = [Shelf::Streaming, Shelf::Local]
        .into_iter()
        .flat_map(|shelf| library.favorite_tracks(shelf).iter().cloned())
        .collect();
    dedupe(&mut tracks);
    sort_by_added_desc(&mut tracks);
    tracks
}

/// The 100 most played songs, by the play counts kept in the database.
fn most_played(library: &Library, history: &History) -> Vec<Track> {
    let map = track_map(library, history);
    most_played_ids()
        .into_iter()
        .filter_map(|id| map.get(&id).cloned())
        .collect()
}

/// The 100 newest songs in the library. On a Saved streaming shelf the tracks
/// are the favorites, so this degrades to the recently liked there.
fn recently_added(library: &Library) -> Vec<Track> {
    let mut tracks: Vec<Track> = [Shelf::Streaming, Shelf::Local]
        .into_iter()
        .flat_map(|shelf| library.state(shelf).tracks().iter().cloned())
        .collect();
    dedupe(&mut tracks);
    sort_by_added_desc(&mut tracks);
    tracks.truncate(100);
    tracks
}

/// The 100 songs played most recently, across every scope.
fn recently_played(library: &Library, history: &History) -> Vec<Track> {
    let map = track_map(library, history);
    recently_played_ids()
        .into_iter()
        .filter_map(|id| map.get(&id).cloned())
        .collect()
}

/// Drops every track after the first with its id. Tracks with no id are kept.
fn dedupe(tracks: &mut Vec<Track>) {
    let mut seen = HashSet::new();
    tracks.retain(|track| match &track.id {
        Some(id) => seen.insert(id.clone()),
        None => true,
    });
}

/// Newest `added_at` first, with tracks that have none last.
fn sort_by_added_desc(tracks: &mut [Track]) {
    tracks.sort_by(|a, b| b.added_at.cmp(&a.added_at));
}

/// The ids of the 100 most played tracks, seeding the counts from `plays` first.
fn most_played_ids() -> Vec<String> {
    let Ok(connection) = storage::Database::standard().open() else {
        log::warn!("smart playlists: cannot open the play database");
        return Vec::new();
    };
    seed_play_counts(&connection);
    query_ids(
        &connection,
        "SELECT track_id FROM play_counts ORDER BY count DESC, last_played_at DESC LIMIT 100",
    )
}

/// The ids of the 100 most recently played tracks, across every scope.
fn recently_played_ids() -> Vec<String> {
    let Ok(connection) = storage::Database::standard().open() else {
        log::warn!("smart playlists: cannot open the play database");
        return Vec::new();
    };
    query_ids(
        &connection,
        "SELECT track_id, MAX(played_at) AS last FROM plays GROUP BY track_id ORDER BY last DESC LIMIT 100",
    )
}

/// Seeds `play_counts` from the surviving plays, creating the table first as a
/// guard (it also lives in the storage schema). Only rows that are missing are
/// added: a row that already exists keeps its count, which the per-play upsert
/// in history keeps going, so pruning the plays log can never lower a count.
fn seed_play_counts(connection: &Connection) {
    if let Err(error) = connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS play_counts (
            scope TEXT NOT NULL,
            provider TEXT NOT NULL,
            track_id TEXT NOT NULL,
            count INTEGER NOT NULL,
            last_played_at INTEGER NOT NULL,
            PRIMARY KEY (scope, provider, track_id)
        );",
    ) {
        log::warn!("smart playlists: cannot create play_counts: {error}");
        return;
    }
    if let Err(error) = connection.execute(
        "INSERT OR IGNORE INTO play_counts (scope, provider, track_id, count, last_played_at)
         SELECT scope, provider, track_id, COUNT(*), MAX(played_at) FROM plays
         GROUP BY scope, provider, track_id",
        [],
    ) {
        log::warn!("smart playlists: cannot seed play counts: {error}");
    }
}

/// The first column of every row a query answers, or an empty list on failure.
fn query_ids(connection: &Connection, sql: &str) -> Vec<String> {
    let mut statement = match connection.prepare(sql) {
        Ok(statement) => statement,
        Err(error) => {
            log::warn!("smart playlists: cannot prepare {sql}: {error}");
            return Vec::new();
        }
    };
    match statement.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
        Err(error) => {
            log::warn!("smart playlists: cannot read {sql}: {error}");
            Vec::new()
        }
    }
}
