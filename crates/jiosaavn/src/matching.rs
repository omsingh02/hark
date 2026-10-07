use crate::models::JioSaavnSong;

/// Computes normalized similarity between 0.0 and 1.0 using token overlap and character distance.
pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn token_similarity(a: &str, b: &str) -> f32 {
    let a_norm = normalize(a);
    let b_norm = normalize(b);

    if a_norm.is_empty() || b_norm.is_empty() {
        return 0.0;
    }

    if a_norm == b_norm {
        return 1.0;
    }

    let a_tokens: Vec<&str> = a_norm.split_whitespace().collect();
    let b_tokens: Vec<&str> = b_norm.split_whitespace().collect();

    let mut matches = 0;
    for token in &a_tokens {
        if b_tokens.contains(token) {
            matches += 1;
        }
    }

    let total = a_tokens.len().max(b_tokens.len()) as f32;
    (matches as f32) / total
}

/// Evaluates a candidate JioSaavnSong against the target title and artist.
/// Returns a score between 0.0 and 1.0.
pub fn score_candidate(candidate: &JioSaavnSong, target_title: &str, target_artist: &str) -> f32 {
    let cand_title = candidate.clean_title();
    let cand_artist = candidate.clean_artist();

    let title_sim = token_similarity(&cand_title, target_title);
    let artist_sim = token_similarity(&cand_artist, target_artist);

    // Baseline weighted score: 60% title, 40% artist
    let mut score = (title_sim * 0.6) + (artist_sim * 0.4);

    let lower_cand = format!(
        "{} {}",
        cand_title.to_lowercase(),
        cand_artist.to_lowercase()
    );
    let lower_target = format!(
        "{} {}",
        target_title.to_lowercase(),
        target_artist.to_lowercase()
    );

    // Penalty for undesirable variations (karaoke, tribute, instrumental, cover)
    // unless the target specifically asked for them
    let noise_words = [
        "karaoke",
        "tribute",
        "instrumental",
        "cover",
        "recreation",
        "parody",
    ];
    for word in &noise_words {
        if lower_cand.contains(word) && !lower_target.contains(word) {
            score *= 0.3; // 70% penalty
        }
    }

    // Small bonus if 320kbps is available
    if candidate.is_320kbps.as_deref() == Some("true") {
        score += 0.05;
    }

    score.min(1.0)
}

/// Selects the best matching candidate from a list of JioSaavn songs.
pub fn select_best_match(
    candidates: Vec<JioSaavnSong>,
    target_title: &str,
    target_artist: &str,
) -> Option<JioSaavnSong> {
    if candidates.is_empty() {
        return None;
    }

    let mut scored: Vec<(f32, JioSaavnSong)> = candidates
        .into_iter()
        .map(|c| {
            let s = score_candidate(&c, target_title, target_artist);
            (s, c)
        })
        .collect();

    // Sort descending by score
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Best candidate must satisfy minimum threshold
    if let Some((score, song)) = scored.into_iter().next() {
        if score >= 0.25 {
            return Some(song);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::JioSaavnSong;

    fn make_song(title: &str, artist: &str, is_320: bool) -> JioSaavnSong {
        JioSaavnSong {
            id: Some("test_id".into()),
            song: Some(title.into()),
            album: Some(title.into()),
            year: Some("2024".into()),
            primary_artists: Some(artist.into()),
            singers: None,
            featured_artists: None,
            label: None,
            language: None,
            copyright_text: None,
            duration: None,
            is_320kbps: Some(if is_320 {
                "true".into()
            } else {
                "false".into()
            }),
            image: None,
            encrypted_media_url: Some("enc_dummy".into()),
            media_preview_url: None,
            perma_url: None,
        }
    }

    #[test]
    fn test_exact_match_scores_highest() {
        let candidates = vec![
            make_song("Wavy (Karaoke Version)", "Karaoke Team", true),
            make_song("Wavy", "Karan Aujla, Jay Trak", true),
            make_song("Different Song", "Karan Aujla", true),
        ];

        let best = select_best_match(candidates, "Wavy", "Karan Aujla");
        assert!(best.is_some());
        let s = best.unwrap();
        assert_eq!(s.song.as_deref(), Some("Wavy"));
        assert_eq!(s.primary_artists.as_deref(), Some("Karan Aujla, Jay Trak"));
    }

    #[test]
    fn test_karaoke_penalized() {
        let karaoke = make_song("Taare Falak Se Aa Gaye (Karaoke)", "Karaoke Studio", true);
        let score = score_candidate(&karaoke, "Taare Falak Se Aa Gaye", "Bappi Lahiri");
        assert!(score < 0.35);
    }
}
