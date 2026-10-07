use jiosaavn::{JioSaavnClient, unescape_html, token_similarity};

#[test]
fn test_html_unescaping() {
    let raw = "Tu Hi Re (From &#039;Bombay&#039;) &amp; &quot;Special&quot;";
    let unescaped = unescape_html(raw);
    assert_eq!(unescaped, "Tu Hi Re (From 'Bombay') & \"Special\"");
}

#[test]
fn test_token_similarity() {
    let sim = token_similarity("Karan Aujla, Jay Trak", "Karan Aujla");
    assert!(sim >= 0.5);

    let sim2 = token_similarity("Wavy (Official Audio)", "Wavy");
    assert!(sim2 >= 0.3);
}

#[tokio::test]
#[ignore = "queries the live JioSaavn API; run with `cargo test -- --ignored`"]
async fn test_live_search_and_stream_resolution() {
    let client = JioSaavnClient::new();
    let song = client.find_best_match("Wavy", "Karan Aujla").await;
    assert!(song.is_ok(), "Failed to find Wavy by Karan Aujla: {:?}", song.err());

    let song = song.unwrap();
    assert!(song.clean_title().to_lowercase().contains("wavy"));

    let stream = client.get_stream_url(&song).await;
    assert!(stream.is_ok(), "Failed to resolve stream URL: {:?}", stream.err());

    let stream_info = stream.unwrap();
    assert!(stream_info.url.starts_with("http"));
    assert!(stream_info.bitrate_kbps >= 96);
}
