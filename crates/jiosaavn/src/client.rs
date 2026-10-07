use reqwest::Client;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::matching::select_best_match;
use crate::models::{AuthTokenResponse, JioSaavnSong, SearchResultsResponse, StreamInfo};
use crate::tagger::tag_m4a_file;

const API_ENDPOINT: &str = "https://www.jiosaavn.com/api.php";
const JIOSAAVN_REFERER: &str = "https://www.jiosaavn.com/";
const BITRATE_WATERFALL: &[&str] = &["320", "160", "128", "96"];

#[derive(Clone)]
pub struct JioSaavnClient {
    client: Client,
}

impl Default for JioSaavnClient {
    fn default() -> Self {
        Self::new()
    }
}

impl JioSaavnClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();
        Self { client }
    }

    /// Searches JioSaavn catalog and returns up to `limit` candidates.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<JioSaavnSong>, String> {
        let n_str = limit.to_string();
        let resp = self
            .client
            .get(API_ENDPOINT)
            .query(&[
                ("__call", "search.getResults"),
                ("_format", "json"),
                ("_marker", "0"),
                ("cc", "in"),
                ("includeMetaTags", "1"),
                ("q", query),
                ("n", &n_str),
            ])
            .send()
            .await
            .map_err(|e| format!("Search request failed: {}", e))?;

        let json_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read search body: {}", e))?;

        let parsed: SearchResultsResponse = serde_json::from_str(&json_text)
            .map_err(|e| format!("JSON parse error (search.getResults): {}", e))?;

        Ok(parsed.results.unwrap_or_default())
    }

    /// Finds the best matching song in the catalog using candidate similarity scoring.
    pub async fn find_best_match(&self, title: &str, artist: &str) -> Result<JioSaavnSong, String> {
        let query = format!("{} {}", title, artist);
        let candidates = self.search(&query, 10).await?;

        select_best_match(candidates, title, artist)
            .ok_or_else(|| format!("No matching track found for '{} - {}'", title, artist))
    }

    /// Resolves the direct CDN audio stream URL using bitrate waterfall (320 -> 160 -> 128 -> 96).
    pub async fn get_stream_url(&self, song: &JioSaavnSong) -> Result<StreamInfo, String> {
        let enc_url = song
            .encrypted_media_url
            .as_deref()
            .ok_or_else(|| "Track has no encrypted_media_url".to_string())?;

        for &bitrate in BITRATE_WATERFALL {
            let resp = self
                .client
                .post(API_ENDPOINT)
                .form(&[
                    ("__call", "song.generateAuthToken"),
                    ("url", enc_url),
                    ("bitrate", bitrate),
                    ("api_version", "4"),
                    ("_format", "json"),
                    ("ctx", "web6dot0"),
                    ("_marker", "0"),
                ])
                .send()
                .await;

            if let Ok(r) = resp {
                if let Ok(text) = r.text().await {
                    if let Ok(auth_resp) = serde_json::from_str::<AuthTokenResponse>(&text) {
                        if let Some(auth_url) = auth_resp.auth_url {
                            if !auth_url.is_empty() {
                                let kbps = bitrate.parse::<u32>().unwrap_or(320);
                                return Ok(StreamInfo {
                                    url: auth_url,
                                    bitrate_kbps: kbps,
                                    format: "m4a".into(),
                                });
                            }
                        }
                    }
                }
            }
        }

        Err(format!(
            "Failed to resolve stream URL for track: {}",
            song.clean_title()
        ))
    }

    /// Downloads and tags a song atomically into `download_dir`.
    pub async fn download_song(
        &self,
        song: &JioSaavnSong,
        download_dir: &Path,
    ) -> Result<PathBuf, String> {
        let stream_info = self.get_stream_url(song).await?;

        tokio::fs::create_dir_all(download_dir).await.map_err(|e| {
            format!(
                "Failed to create download dir {}: {}",
                download_dir.display(),
                e
            )
        })?;

        let clean_t = song.clean_title();
        let clean_a = song.clean_artist();

        let safe_title = clean_t.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let safe_artist = clean_a.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let filename = format!("{} - {}.m4a", safe_title, safe_artist);
        let part_filename = format!("{}.part", filename);

        let final_path = download_dir.join(&filename);
        let part_path = download_dir.join(&part_filename);

        // Stream download chunk-by-chunk to .part temporary file (no full-buffer in RAM)
        let resp = self
            .client
            .get(&stream_info.url)
            .header("Referer", JIOSAAVN_REFERER)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch stream payload: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Stream CDN returned HTTP {}", resp.status()));
        }

        let mut file = tokio::fs::File::create(&part_path)
            .await
            .map_err(|e| format!("Failed to create temp file {}: {}", part_path.display(), e))?;

        let mut resp = resp;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("Download stream error: {}", e))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Failed to write chunk: {}", e))?;
        }
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush temp file: {}", e))?;
        drop(file);

        // Fetch Cover Art (High-Res)
        let mut cover_bytes = None;
        if let Some(hq_cover_url) = song.high_res_cover_url() {
            if let Ok(resp) = self.client.get(&hq_cover_url).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        cover_bytes = Some(bytes.to_vec());
                    }
                }
            }
        }

        // Apply metadata tags to .part file — propagate errors instead of silently discarding
        tag_m4a_file(&part_path, song, cover_bytes.as_deref())
            .map_err(|e| format!("Tagging failed for {}: {}", part_path.display(), e))?;

        // Atomically rename .part to final destination
        tokio::fs::rename(&part_path, &final_path)
            .await
            .map_err(|e| {
                format!(
                    "Failed to rename {} to {}: {}",
                    part_path.display(),
                    final_path.display(),
                    e
                )
            })?;

        Ok(final_path)
    }

    /// Fast lookup: checks if matching file is already cached; if not, downloads and tags.
    pub async fn ensure_song(
        &self,
        title: &str,
        artist: &str,
        download_dir: &Path,
    ) -> Result<PathBuf, String> {
        let search_t = title.trim().to_lowercase();
        if let Ok(mut entries) = tokio::fs::read_dir(download_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("m4a") {
                    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if file_stem.to_lowercase().contains(&search_t) {
                            return Ok(path);
                        }
                    }
                }
            }
        }

        let song = self.find_best_match(title, artist).await?;
        self.download_song(&song, download_dir).await
    }

    /// Dispatches the track to the user's default player (MPD / mpc), falling back to xdg-open.
    pub async fn play_in_default_player(&self, path: &Path) -> Result<String, String> {
        if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
            let rel_path = format!("ShazamLive/{}", file_name);

            let _ = tokio::process::Command::new("mpc")
                .args(["update", "--wait", "ShazamLive"])
                .status()
                .await;

            let insert_ok = tokio::process::Command::new("mpc")
                .args(["insert", &rel_path])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if insert_ok {
                let _ = tokio::process::Command::new("mpc")
                    .arg("next")
                    .status()
                    .await;
                return Ok(format!("Sent to default player (MPD): {}", rel_path));
            }
        }

        let _ = tokio::process::Command::new("xdg-open")
            .arg(path)
            .status()
            .await;
        Ok(format!("Sent to default player: {}", path.display()))
    }

    /// Resolves the XDG Music directory following freedesktop.org specification ($XDG_MUSIC_DIR/ShazamLive).
    pub fn get_music_dir() -> PathBuf {
        dirs::audio_dir()
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                PathBuf::from(home).join("Music")
            })
            .join("ShazamLive")
    }
}
