use hark::audio::{AudioCapture, AudioSourceMode, SilenceDetector};
use hark::dsp::SignatureGenerator;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("=== hark live capture and Shazam API check ===");
    let capture = AudioCapture::new(AudioSourceMode::Auto)?;
    let silence = SilenceDetector::default();
    let sig_gen = SignatureGenerator::new();

    println!("Capturing audio from microphone for 6 seconds...");
    for i in 1..=6 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        println!(
            "  Elapsed: {}s, ring buffer sample count: {}",
            i,
            capture.sample_count()
        );
    }

    let samples = capture.extract_chunk(6);
    println!("Extracted samples: {}", samples.len());

    // Save raw PCM i16 to /tmp/captured_live.pcm
    let pcm_bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let _ = std::fs::write("/tmp/captured_live.pcm", &pcm_bytes);
    println!(
        "Saved {} bytes raw PCM to /tmp/captured_live.pcm",
        pcm_bytes.len()
    );

    let (is_silent, dbfs) = silence.is_silent(&samples);
    println!(
        "Silence check: is_silent={}, dbfs={:.2} dBFS",
        is_silent, dbfs
    );

    let sig_uri = sig_gen.generate_from_i16(&samples);
    match sig_uri {
        Some(uri) => {
            println!("Signature generated successfully (len: {})", uri.len());
            println!("URI sample: {}...", &uri[..60.min(uri.len())]);

            let client = Client::new();
            let now_sec = chrono::Utc::now().timestamp();
            let sample_ms = (samples.len() as f32 / 16.0) as u32;

            let payload = json!({
                "geolocation": {
                    "altitude": 216,
                    "latitude": 28.6139,
                    "longitude": 77.2090
                },
                "signature": {
                    "samplems": sample_ms,
                    "timestamp": now_sec,
                    "uri": uri
                },
                "timestamp": now_sec,
                "timezone": "Asia/Kolkata"
            });

            let uuid1 = Uuid::new_v4().to_string().to_uppercase();
            let uuid2 = Uuid::new_v4().to_string();
            let url = format!(
                "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3",
                uuid1, uuid2
            );

            println!("Sending POST to amp.shazam.com...");
            let start = std::time::Instant::now();
            let response = client
                .post(&url)
                .header(
                    "User-Agent",
                    "Dalvik/2.1.0 (Linux; U; Android 6.0.1; SM-G920F Build/MMB29K)",
                )
                .header("Content-Type", "application/json")
                .header("Content-Language", "en_US")
                .json(&payload)
                .send()
                .await?;

            let duration = start.elapsed();
            let status = response.status();
            println!("HTTP Status: {} in {:.3}s", status, duration.as_secs_f32());
            let body = response.text().await?;
            println!(
                "Raw Response Body (first 1000 chars):\n{}",
                &body[..1000.min(body.len())]
            );
        }
        None => {
            println!("Failed to generate signature from samples.");
        }
    }

    Ok(())
}
