use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShazamResponse {
    pub matches: Option<Vec<MatchItem>>,
    pub track: Option<TrackItem>,
    pub tagid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchItem {
    pub id: Option<String>,
    pub offset: Option<f64>,
    pub frequencyskew: Option<f64>,
    pub timeskew: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackItem {
    pub key: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub isrc: Option<String>,
    pub images: Option<TrackImages>,
    pub share: Option<TrackShare>,
    pub hub: Option<TrackHub>,
    pub sections: Option<Vec<TrackSection>>,
    pub genres: Option<TrackGenres>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackImages {
    pub background: Option<String>,
    pub coverart: Option<String>,
    pub coverarthq: Option<String>,
    pub joecolor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackShare {
    pub subject: Option<String>,
    pub text: Option<String>,
    pub href: Option<String>,
    pub image: Option<String>,
    pub twitter: Option<String>,
    pub html: Option<String>,
    pub avatar: Option<String>,
    pub snapchat: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackHub {
    pub actions: Option<Vec<HubAction>>,
    pub options: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubAction {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub action_type: Option<String>,
    pub id: Option<String>,
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSection {
    #[serde(rename = "type")]
    pub section_type: Option<String>,
    pub text: Option<Vec<String>>,
    pub metadata: Option<Vec<SectionMetadata>>,
    pub youtubeurl: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionMetadata {
    pub title: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackGenres {
    pub primary: Option<String>,
}

/// Consolidated high-level domain representation of an identified song.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecognizedSong {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub isrc: Option<String>,
    pub shazam_key: Option<String>,
    pub cover_art_url: Option<String>,
    pub cover_art_hq_url: Option<String>,
    pub offset_seconds: Option<f64>,
    pub preview_audio_url: Option<String>,
    pub youtube_url: Option<String>,
    pub share_url: Option<String>,
    pub lyrics: Option<Vec<String>>,
}

impl RecognizedSong {
    /// Extracts a consolidated `RecognizedSong` domain model from an upstream raw `ShazamResponse`.
    pub fn from_shazam_response(resp: ShazamResponse) -> Option<Self> {
        let track = resp.track?;
        let title = track.title.unwrap_or_else(|| "Unknown Title".to_string());
        let artist = track.subtitle.unwrap_or_else(|| "Unknown Artist".to_string());

        let mut album = None;
        let mut lyrics = None;
        let mut youtube_url = None;

        if let Some(sections) = track.sections {
            for section in sections {
                if section.section_type.as_deref() == Some("SONG") {
                    if let Some(meta) = section.metadata {
                        for m in meta {
                            if m.title.as_deref().map(|t| t.eq_ignore_ascii_case("album")).unwrap_or(false) {
                                album = m.text;
                            }
                        }
                    }
                } else if section.section_type.as_deref() == Some("LYRICS") {
                    lyrics = section.text;
                } else if section.section_type.as_deref() == Some("VIDEO") {
                    if let Some(yt) = section.youtubeurl {
                        if let Some(actions) = yt.get("actions").and_then(|a| a.as_array()) {
                            for act in actions {
                                if let Some(uri) = act.get("uri").and_then(|u| u.as_str()) {
                                    youtube_url = Some(uri.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        let genre = track.genres.and_then(|g| g.primary);
        let cover_art_url = track.images.as_ref().and_then(|img| img.coverart.clone());
        let cover_art_hq_url = track.images.as_ref().and_then(|img| img.coverarthq.clone().or_else(|| img.coverart.clone()));

        let offset_seconds = resp.matches.as_ref().and_then(|m| m.first()).and_then(|m| m.offset);

        let preview_audio_url = track.hub.as_ref().and_then(|hub| {
            hub.actions.as_ref().and_then(|actions| {
                actions.iter().find_map(|act| {
                    if act.action_type.as_deref() == Some("uri")
                        && act.uri.as_deref().map(|u| u.ends_with(".m4a") || u.contains("itunes.apple.com")).unwrap_or(false)
                    {
                        act.uri.clone()
                    } else {
                        None
                    }
                })
            })
        });

        let share_url = track.share.and_then(|s| s.href).or(track.url);

        Some(Self {
            title,
            artist,
            album,
            genre,
            isrc: track.isrc,
            shazam_key: track.key,
            cover_art_url,
            cover_art_hq_url,
            offset_seconds,
            preview_audio_url,
            youtube_url,
            share_url,
            lyrics,
        })
    }

    pub fn display_id(&self) -> String {
        format!("{} - {}", self.title, self.artist)
    }
}
