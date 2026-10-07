use std::fs::{create_dir_all, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::network::models::RecognizedSong;

pub struct HistoryStorage {
    txt_path: PathBuf,
    jsonl_path: PathBuf,
}

impl HistoryStorage {
    pub fn new() -> Self {
        Self::open(&dirs_or_fallback())
    }

    /// Opens the history stored under `<data_dir>/hark`, creating it if needed.
    ///
    /// On the very first run, a history written by shazam-daemon (`<data_dir>/shazam_history.*`)
    /// is copied in; the original files are left untouched. Later runs never import again, so
    /// clearing the history stays cleared.
    fn open(data_dir: &Path) -> Self {
        let dir = data_dir.join("hark");
        let first_run = !dir.exists();
        let _ = create_dir_all(&dir);

        let txt_path = dir.join("history.txt");
        let jsonl_path = dir.join("history.jsonl");

        if first_run {
            Self::import_legacy(&data_dir.join("shazam_history.txt"), &txt_path);
            Self::import_legacy(&data_dir.join("shazam_history.jsonl"), &jsonl_path);
        }

        let storage = Self {
            txt_path,
            jsonl_path,
        };

        // Automatically compact existing history on initialization so any past duplicates are cleaned up
        storage.compact_history();

        storage
    }

    fn import_legacy(legacy: &Path, target: &Path) {
        if !target.exists() && legacy.is_file() {
            let _ = std::fs::copy(legacy, target);
        }
    }

    /// Checks if a stored JSON record matches a RecognizedSong instance
    pub fn is_same_song(stored: &serde_json::Value, song: &RecognizedSong) -> bool {
        // 1. Match by Apple Shazam key
        let stored_key = stored.get("shazam_key").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !stored_key.is_empty() {
            if let Some(ref song_key) = song.shazam_key {
                if stored_key == song_key.trim() {
                    return true;
                }
            }
        }

        // 2. Match by standard ISRC code
        let stored_isrc = stored.get("isrc").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !stored_isrc.is_empty() {
            if let Some(ref song_isrc) = song.isrc {
                if stored_isrc == song_isrc.trim() {
                    return true;
                }
            }
        }

        // 3. Match by normalized (title, artist)
        let stored_title = stored.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let stored_artist = stored.get("artist").and_then(|v| v.as_str()).unwrap_or("");

        if !stored_title.is_empty() && !song.title.is_empty() {
            if normalize_field(stored_title) == normalize_field(&song.title)
                && normalize_field(stored_artist) == normalize_field(&song.artist)
            {
                return true;
            }

            // 4. Canonical Family Match (Base title + Lead artist)
            let stored_base = extract_base_title(stored_title);
            let song_base = extract_base_title(&song.title);
            let stored_lead = extract_lead_artist(stored_artist);
            let song_lead = extract_lead_artist(&song.artist);

            if !stored_base.is_empty() && !song_base.is_empty() && stored_base == song_base {
                if !stored_lead.is_empty() && !song_lead.is_empty() && stored_lead == song_lead {
                    return true;
                }
            }
        }

        false
    }

    /// Checks if two JSON records represent the same song
    pub fn are_same_entries(a: &serde_json::Value, b: &serde_json::Value) -> bool {
        let a_key = a.get("shazam_key").and_then(|v| v.as_str()).unwrap_or("").trim();
        let b_key = b.get("shazam_key").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !a_key.is_empty() && !b_key.is_empty() && a_key == b_key {
            return true;
        }

        let a_isrc = a.get("isrc").and_then(|v| v.as_str()).unwrap_or("").trim();
        let b_isrc = b.get("isrc").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !a_isrc.is_empty() && !b_isrc.is_empty() && a_isrc == b_isrc {
            return true;
        }

        let a_title = a.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let b_title = b.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let a_artist = a.get("artist").and_then(|v| v.as_str()).unwrap_or("");
        let b_artist = b.get("artist").and_then(|v| v.as_str()).unwrap_or("");

        if !a_title.is_empty() && !b_title.is_empty() {
            if normalize_field(a_title) == normalize_field(b_title)
                && normalize_field(a_artist) == normalize_field(b_artist)
            {
                return true;
            }

            let a_base = extract_base_title(a_title);
            let b_base = extract_base_title(b_title);
            let a_lead = extract_lead_artist(a_artist);
            let b_lead = extract_lead_artist(b_artist);

            if !a_base.is_empty() && !b_base.is_empty() && a_base == b_base {
                if !a_lead.is_empty() && !b_lead.is_empty() && a_lead == b_lead {
                    return true;
                }
            }
        }

        false
    }

    /// Logs a recognized song while strictly keeping only the latest identified record.
    /// If the song already exists in history, the previous entry is removed and the song
    /// is bumped to the top (latest position) with its updated timestamp.
    pub fn log_song(&self, song: &RecognizedSong) {
        let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut entries = self.read_all_entries();

        // Remove any previous occurrence of this song
        entries.retain(|e| !Self::is_same_song(e, song));

        let new_entry = serde_json::json!({
            "timestamp": now_str,
            "title": song.title,
            "artist": song.artist,
            "album": song.album.as_deref().unwrap_or(""),
            "genre": song.genre.as_deref().unwrap_or(""),
            "isrc": song.isrc.as_deref().unwrap_or(""),
            "shazam_key": song.shazam_key.as_deref().unwrap_or(""),
            "cover_art": song.cover_art_hq_url.as_deref().or(song.cover_art_url.as_deref()).unwrap_or(""),
            "offset": song.offset_seconds.unwrap_or(0.0),
            "preview_url": song.preview_audio_url.as_deref().unwrap_or(""),
            "youtube_url": song.youtube_url.as_deref().unwrap_or(""),
            "share_url": song.share_url.as_deref().unwrap_or(""),
            "lyrics": song.lyrics.as_deref().unwrap_or(&[])
        });

        // Append to the end (latest position)
        entries.push(new_entry);

        self.atomic_save(&entries);
    }

    /// Reads all JSONL records currently stored
    fn read_all_entries(&self) -> Vec<serde_json::Value> {
        if !self.jsonl_path.exists() {
            return Vec::new();
        }

        let Ok(file) = OpenOptions::new().read(true).open(&self.jsonl_path) else {
            return Vec::new();
        };

        let reader = BufReader::new(file);
        reader
            .lines()
            .filter_map(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
            .collect()
    }

    /// Compacts existing history to keep only the latest record of each song.
    pub fn compact_history(&self) {
        if !self.jsonl_path.exists() {
            return;
        }

        let raw_entries = self.read_all_entries();
        if raw_entries.is_empty() {
            return;
        }

        let mut deduped: Vec<serde_json::Value> = Vec::with_capacity(raw_entries.len());
        for entry in raw_entries {
            // Remove earlier duplicate of this song if already present
            deduped.retain(|e| !Self::are_same_entries(e, &entry));
            // Append latest occurrence
            deduped.push(entry);
        }

        self.atomic_save(&deduped);
    }

    /// Atomically persists entries to both JSONL and TXT formats using temp files and fs::rename
    fn atomic_save(&self, entries: &[serde_json::Value]) {
        // 1. Atomic write to JSONL
        let temp_jsonl = self.jsonl_path.with_extension("jsonl.tmp");
        if let Ok(mut file) = OpenOptions::new().create(true).write(true).truncate(true).open(&temp_jsonl) {
            for entry in entries {
                if let Ok(serialized) = serde_json::to_string(entry) {
                    let _ = writeln!(file, "{}", serialized);
                }
            }
            let _ = file.flush();
            drop(file);
            let _ = std::fs::rename(&temp_jsonl, &self.jsonl_path);
        }

        // 2. Atomic write to TXT
        let temp_txt = self.txt_path.with_extension("txt.tmp");
        if let Ok(mut file) = OpenOptions::new().create(true).write(true).truncate(true).open(&temp_txt) {
            for entry in entries {
                let ts = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                let a = entry.get("artist").and_then(|v| v.as_str()).unwrap_or("");
                let t = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let _ = writeln!(file, "[{}] {} - {}", ts, a, t);
            }
            let _ = file.flush();
            drop(file);
            let _ = std::fs::rename(&temp_txt, &self.txt_path);
        }
    }

    pub fn get_recent(&self, limit: usize) -> Vec<serde_json::Value> {
        let mut entries = self.read_all_entries();
        entries.reverse();
        entries.into_iter().take(limit).collect()
    }

    pub fn search(&self, query: &str) -> Vec<serde_json::Value> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.get_recent(50);
        }

        let mut entries = self.read_all_entries();
        entries.reverse();

        entries
            .into_iter()
            .filter(|item| {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let artist = item.get("artist").and_then(|v| v.as_str()).unwrap_or("");
                let album = item.get("album").and_then(|v| v.as_str()).unwrap_or("");
                let genre = item.get("genre").and_then(|v| v.as_str()).unwrap_or("");
                title.to_lowercase().contains(&q)
                    || artist.to_lowercase().contains(&q)
                    || album.to_lowercase().contains(&q)
                    || genre.to_lowercase().contains(&q)
            })
            .take(50)
            .collect()
    }

    pub fn delete_entry(&self, key_or_title: &str, artist: Option<&str>) -> bool {
        let mut entries = self.read_all_entries();
        let initial_len = entries.len();
        let target_key = key_or_title.trim();
        let target_title = normalize_field(key_or_title);
        let target_artist = artist.map(normalize_field);

        entries.retain(|e| {
            // Match by shazam_key if provided
            if let Some(k) = e.get("shazam_key").and_then(|v| v.as_str()) {
                if !k.is_empty() && k.trim() == target_key {
                    return false;
                }
            }
            // Match by title (and artist if provided)
            let t = e.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let a = e.get("artist").and_then(|v| v.as_str()).unwrap_or("");
            if normalize_field(t) == target_title {
                if let Some(ref req_a) = target_artist {
                    if normalize_field(a) == *req_a {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            true
        });

        if entries.len() < initial_len {
            self.atomic_save(&entries);
            true
        } else {
            false
        }
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.jsonl_path);
        let _ = std::fs::remove_file(&self.txt_path);
    }
}

pub fn extract_base_title(title: &str) -> String {
    let mut s = title.to_lowercase();

    // 1. Remove bracketed/parenthetical content: (...), [...], {...}
    while let Some(start) = s.find(|c| c == '(' || c == '[' || c == '{') {
        if let Some(end) = s[start..].find(|c| c == ')' || c == ']' || c == '}') {
            s.replace_range(start..=start + end, " ");
        } else {
            s.truncate(start);
            break;
        }
    }

    // 2. Cut at common splitters like " x ", " vs ", " feat ", " ft ", " - "
    for delim in &[" x ", " vs ", " feat ", " feat. ", " ft ", " ft. ", " - "] {
        if let Some(pos) = s.find(delim) {
            s.truncate(pos);
        }
    }

    // 3. Normalize alphanumeric tokens
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn extract_lead_artist(artist: &str) -> String {
    let mut s = artist.to_lowercase();
    for delim in &[" & ", " and ", ",", " feat ", " feat. ", " ft ", " ft. ", " x ", " vs "] {
        if let Some(pos) = s.find(delim) {
            s.truncate(pos);
        }
    }
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_field(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn dirs_or_fallback() -> PathBuf {
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(data_home)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share")
    } else {
        PathBuf::from("/tmp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_and_artist() {
        assert_eq!(extract_base_title("Wavy x Baller (The Gangsters)"), "wavy");
        assert_eq!(extract_base_title("Wavy"), "wavy");
        assert_eq!(
            extract_lead_artist("Karan Aujla, Shubh, Sidhu Moose Wala & iamnjan..."),
            "karan aujla"
        );
        assert_eq!(extract_lead_artist("Karan Aujla & Jay Trak"), "karan aujla");
    }

    #[test]
    fn test_remix_mashup_matching() {
        let entry1 = serde_json::json!({
            "title": "Wavy x Baller (The Gangsters)",
            "artist": "Karan Aujla, Shubh, Sidhu Moose Wala & iamnjan...",
            "shazam_key": "12345"
        });
        let entry2 = serde_json::json!({
            "title": "Wavy",
            "artist": "Karan Aujla & Jay Trak",
            "shazam_key": "67890"
        });

        assert!(HistoryStorage::are_same_entries(&entry1, &entry2));
    }

    #[test]
    fn test_delete_entry() {
        let temp_dir = std::env::temp_dir().join(format!("hark_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let storage = HistoryStorage {
            txt_path: temp_dir.join("test_history.txt"),
            jsonl_path: temp_dir.join("test_history.jsonl"),
        };

        let song1 = RecognizedSong {
            title: "Song One".to_string(),
            artist: "Artist A".to_string(),
            album: None,
            genre: None,
            cover_art_url: None,
            cover_art_hq_url: None,
            shazam_key: Some("11111".to_string()),
            isrc: None,
            offset_seconds: None,
            preview_audio_url: None,
            youtube_url: None,
            share_url: None,
            lyrics: None,
        };
        let song2 = RecognizedSong {
            title: "Song Two".to_string(),
            artist: "Artist B".to_string(),
            album: None,
            genre: None,
            cover_art_url: None,
            cover_art_hq_url: None,
            shazam_key: Some("22222".to_string()),
            isrc: None,
            offset_seconds: None,
            preview_audio_url: None,
            youtube_url: None,
            share_url: None,
            lyrics: None,
        };

        storage.log_song(&song1);
        storage.log_song(&song2);
        assert_eq!(storage.get_recent(10).len(), 2);

        // Delete song1 by shazam_key
        assert!(storage.delete_entry("11111", None));
        assert_eq!(storage.get_recent(10).len(), 1);

        // Delete song2 by title & artist
        assert!(storage.delete_entry("Song Two", Some("Artist B")));
        assert_eq!(storage.get_recent(10).len(), 0);

        // Deleting non-existent returns false
        assert!(!storage.delete_entry("Non Existent", None));

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    fn scratch_data_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hark_{}_{}_{}", tag, std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_open_stores_history_under_hark_dir() {
        let data = scratch_data_dir("layout");
        let storage = HistoryStorage::open(&data);
        assert_eq!(storage.jsonl_path, data.join("hark").join("history.jsonl"));
        assert_eq!(storage.txt_path, data.join("hark").join("history.txt"));
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn test_first_run_copies_legacy_history_and_keeps_original() {
        let data = scratch_data_dir("legacy");
        let line = r#"{"timestamp":"2026-01-01 00:00:00","title":"Old Song","artist":"Old Artist","album":"","genre":"","isrc":"","shazam_key":"42","cover_art":"","offset":0.0,"preview_url":"","youtube_url":"","share_url":"","lyrics":[]}"#;
        std::fs::write(data.join("shazam_history.jsonl"), format!("{line}\n")).unwrap();
        std::fs::write(data.join("shazam_history.txt"), "[2026-01-01 00:00:00] Old Artist - Old Song\n").unwrap();

        let storage = HistoryStorage::open(&data);
        let recent = storage.get_recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0]["title"], "Old Song");
        assert!(data.join("shazam_history.jsonl").exists(), "legacy file must be left in place");

        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn test_legacy_history_is_not_imported_again_after_clear() {
        let data = scratch_data_dir("noreimport");
        let line = r#"{"timestamp":"2026-01-01 00:00:00","title":"Old Song","artist":"Old Artist","album":"","genre":"","isrc":"","shazam_key":"42","cover_art":"","offset":0.0,"preview_url":"","youtube_url":"","share_url":"","lyrics":[]}"#;
        std::fs::write(data.join("shazam_history.jsonl"), format!("{line}\n")).unwrap();

        let storage = HistoryStorage::open(&data);
        assert_eq!(storage.get_recent(10).len(), 1);
        storage.clear();

        let reopened = HistoryStorage::open(&data);
        assert!(reopened.get_recent(10).is_empty());

        let _ = std::fs::remove_dir_all(data);
    }
}
