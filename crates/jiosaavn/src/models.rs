use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResultsResponse {
    pub results: Option<Vec<JioSaavnSong>>,
    pub total: Option<u64>,
    pub start: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JioSaavnSong {
    pub id: Option<String>,
    pub song: Option<String>,
    pub album: Option<String>,
    pub year: Option<String>,
    pub primary_artists: Option<String>,
    pub singers: Option<String>,
    pub featured_artists: Option<String>,
    pub label: Option<String>,
    pub language: Option<String>,
    pub copyright_text: Option<String>,
    pub duration: Option<String>,
    #[serde(rename = "320kbps")]
    pub is_320kbps: Option<String>,
    pub image: Option<String>,
    pub encrypted_media_url: Option<String>,
    pub media_preview_url: Option<String>,
    pub perma_url: Option<String>,
}

impl JioSaavnSong {
    pub fn clean_title(&self) -> String {
        let raw = self.song.as_deref().unwrap_or("Unknown Title");
        unescape_html(raw)
    }

    pub fn clean_artist(&self) -> String {
        let raw = self
            .primary_artists
            .as_deref()
            .or(self.singers.as_deref())
            .unwrap_or("Unknown Artist");
        unescape_html(raw)
    }

    pub fn clean_album(&self) -> Option<String> {
        self.album.as_deref().map(unescape_html)
    }

    pub fn high_res_cover_url(&self) -> Option<String> {
        self.image.as_ref().map(|u| {
            u.replace("150x150.jpg", "500x500.jpg")
                .replace("50x50.jpg", "500x500.jpg")
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthTokenResponse {
    pub auth_url: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub url: String,
    pub bitrate_kbps: u32,
    pub format: String,
}

/// Robust HTML entity decoder for JioSaavn strings (e.g. &quot;, &#039;, &amp;)
pub fn unescape_html(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}
