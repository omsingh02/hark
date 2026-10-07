use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use zbus::interface;
use zbus::zvariant::Value;

use crate::cache::CoverCacheManager;
use crate::downloader::JioSaavnClient;
use crate::history::HistoryStorage;
use crate::network::models::RecognizedSong;

pub struct HarkRoot;

#[interface(name = "org.mpris.MediaPlayer2")]
impl HarkRoot {
    #[zbus(property)]
    async fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn identity(&self) -> String {
        "Hark".to_string()
    }

    #[zbus(property)]
    async fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["http".into(), "https".into()]
    }

    #[zbus(property)]
    async fn supported_mime_types(&self) -> Vec<String> {
        vec!["audio/vnd.shazam.sig".into()]
    }

    async fn raise(&self) {}
    async fn quit(&self) {}
}

pub struct HarkPlayer {
    is_listening: Arc<AtomicBool>,
    engine_status: Arc<RwLock<String>>,
    current_song: Arc<RwLock<Option<RecognizedSong>>>,
    downloader: Arc<JioSaavnClient>,
    is_foreground: Arc<AtomicBool>,
    foreground_notify: Arc<tokio::sync::Notify>,
    playback_anchor: Arc<RwLock<Option<(f64, std::time::Instant)>>>,
    history: Arc<HistoryStorage>,
    cover_cache: Arc<CoverCacheManager>,
}

impl HarkPlayer {
    pub fn new(
        is_listening: Arc<AtomicBool>,
        engine_status: Arc<RwLock<String>>,
        current_song: Arc<RwLock<Option<RecognizedSong>>>,
        is_foreground: Arc<AtomicBool>,
        foreground_notify: Arc<tokio::sync::Notify>,
        playback_anchor: Arc<RwLock<Option<(f64, std::time::Instant)>>>,
        history: Arc<HistoryStorage>,
        cover_cache: Arc<CoverCacheManager>,
    ) -> Self {
        Self {
            is_listening,
            engine_status,
            current_song,
            downloader: Arc::new(JioSaavnClient::new()),
            is_foreground,
            foreground_notify,
            playback_anchor,
            history,
            cover_cache,
        }
    }

    /// Enriches history items with local cover art URI if cached on disk
    fn enrich_history_items(&self, items: &mut [serde_json::Value]) {
        for item in items {
            if let Some(obj) = item.as_object_mut() {
                let key = obj.get("shazam_key").and_then(|v| v.as_str()).unwrap_or("").trim();
                if !key.is_empty() && key != "0" {
                    if let Some(local_uri) = self.cover_cache.get_local_uri(key) {
                        obj.insert("local_cover".to_string(), serde_json::Value::String(local_uri));
                    }
                }
            }
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl HarkPlayer {
    #[zbus(property)]
    async fn playback_status(&self) -> String {
        if self.is_listening.load(Ordering::Relaxed) {
            "Playing".to_string()
        } else {
            "Paused".to_string()
        }
    }

    #[zbus(property)]
    async fn engine_status(&self) -> String {
        self.engine_status.read().await.clone()
    }

    #[zbus(property)]
    async fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn position(&self) -> i64 {
        let guard = self.playback_anchor.read().await;
        if let Some((offset_sec, instant)) = *guard {
            let current_sec = offset_sec + instant.elapsed().as_secs_f64();
            (current_sec * 1_000_000.0) as i64
        } else {
            0
        }
    }

    #[zbus(property)]
    async fn can_seek(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_go_next(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_go_previous(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn metadata(&self) -> HashMap<String, Value<'static>> {
        let mut map = HashMap::new();
        let song_guard = self.current_song.read().await;

        if let Some(song) = song_guard.as_ref() {
            let track_id = format!(
                "/org/mpris/MediaPlayer2/Track/{}",
                song.shazam_key.as_deref().filter(|k| !k.is_empty() && *k != "0").unwrap_or("active")
            );
            map.insert("mpris:trackid".into(), Value::from(track_id));
            map.insert("hark:engineStatus".into(), Value::from(self.engine_status.read().await.clone()));
            map.insert("xesam:title".into(), Value::from(song.title.clone()));
            map.insert("xesam:artist".into(), Value::from(vec![song.artist.clone()]));

            if let Some(album) = &song.album {
                map.insert("xesam:album".into(), Value::from(album.clone()));
            }
            if let Some(genre) = &song.genre {
                map.insert("xesam:genre".into(), Value::from(vec![genre.clone()]));
            }
            let art_url = if let Some(key) = song.shazam_key.as_deref().filter(|k| !k.is_empty() && *k != "0") {
                if let Some(local_uri) = self.cover_cache.get_local_uri(key) {
                    Some(local_uri)
                } else {
                    song.cover_art_hq_url.as_ref().or(song.cover_art_url.as_ref()).cloned()
                }
            } else {
                song.cover_art_hq_url.as_ref().or(song.cover_art_url.as_ref()).cloned()
            };
            if let Some(art) = art_url {
                map.insert("mpris:artUrl".into(), Value::from(art.clone()));
                map.insert("xesam:artUrl".into(), Value::from(art));
            }
            if let Some(isrc) = &song.isrc {
                map.insert("hark:isrc".into(), Value::from(isrc.clone()));
            }
            let anchor_guard = self.playback_anchor.read().await;
            let current_offset = if let Some((offset_sec, instant)) = *anchor_guard {
                offset_sec + instant.elapsed().as_secs_f64()
            } else {
                song.offset_seconds.unwrap_or(0.0)
            };
            map.insert("hark:offset".into(), Value::from(current_offset));
            if let Some(yt) = &song.youtube_url {
                map.insert("hark:youtubeUrl".into(), Value::from(yt.clone()));
            }
            if let Some(share) = &song.share_url {
                map.insert("hark:shareUrl".into(), Value::from(share.clone()));
            }
            if let Some(lyrics) = &song.lyrics {
                map.insert("hark:lyrics".into(), Value::from(lyrics.join("\n")));
            }
        } else {
            map.insert("hark:engineStatus".into(), Value::from(self.engine_status.read().await.clone()));
            map.insert(
                "mpris:trackid".into(),
                Value::from("/org/mpris/MediaPlayer2/Track/none".to_string()),
            );
        }

        map
    }

    async fn play(&self) {
        if !self.is_listening.load(Ordering::Relaxed) {
            unsafe { libc::kill(libc::getpid(), libc::SIGUSR1); }
        }
    }

    async fn pause(&self) {
        if self.is_listening.load(Ordering::Relaxed) {
            unsafe { libc::kill(libc::getpid(), libc::SIGUSR1); }
        }
    }

    async fn play_pause(&self) {
        unsafe {
            libc::kill(libc::getpid(), libc::SIGUSR1);
        }
    }

    async fn stop(&self) {
        if self.is_listening.load(Ordering::Relaxed) {
            unsafe { libc::kill(libc::getpid(), libc::SIGUSR1); }
        }
    }

    async fn download_current(&self) -> String {
        let (title, artist) = {
            let guard = self.current_song.read().await;
            match guard.as_ref() {
                Some(s) => (s.title.clone(), s.artist.clone()),
                None => return "Error: No song currently recognized".to_string(),
            }
        };

        let download_dir = JioSaavnClient::get_music_dir();

        let song = match self.downloader.find_best_match(&title, &artist).await {
            Ok(s) => s,
            Err(e) => return format!("Error: {}", e),
        };
        match self.downloader.download_song(&song, &download_dir).await {
            Ok(p) => format!("Success: Downloaded to {}", p.display()),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn download_track(&self, title: String, artist: String) -> String {
        let download_dir = JioSaavnClient::get_music_dir();

        let song = match self.downloader.find_best_match(&title, &artist).await {
            Ok(s) => s,
            Err(e) => return format!("Error: {}", e),
        };
        match self.downloader.download_song(&song, &download_dir).await {
            Ok(p) => format!("Success: Downloaded to {}", p.display()),
            Err(e) => format!("Error: {}", e),
        }
    }

    async fn get_stream_url(&self, title: String, artist: String) -> String {
        let song = match self.downloader.find_best_match(&title, &artist).await {
            Ok(s) => s,
            Err(_) => return String::new(),
        };
        match self.downloader.get_stream_url(&song).await {
            Ok(info) => info.url,
            Err(_) => String::new(),
        }
    }

    async fn get_recent_history(&self, limit: u32) -> String {
        let mut items = self.history.get_recent(limit as usize);
        self.enrich_history_items(&mut items);
        serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
    }

    async fn clear_history(&self) -> bool {
        self.history.clear();
        true
    }

    async fn delete_history_entry(&self, key_or_title: String, artist: String) -> bool {
        let artist_opt = if artist.trim().is_empty() { None } else { Some(artist.as_str()) };
        self.history.delete_entry(&key_or_title, artist_opt)
    }

    async fn search_history(&self, query: String) -> String {
        let mut items = self.history.search(&query);
        self.enrich_history_items(&mut items);
        serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
    }

    async fn set_foreground(&self, in_foreground: bool) -> bool {
        self.is_foreground.store(in_foreground, Ordering::Relaxed);
        if in_foreground {
            self.foreground_notify.notify_waiters();
        }
        true
    }
}
