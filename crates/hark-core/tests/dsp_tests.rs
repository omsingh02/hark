use byteorder::{LittleEndian, ReadBytesExt};
use crc32fast::Hasher;
use hark_core::dsp::{SignatureGenerator, DATA_URI_PREFIX};
use hark_core::models::{RecognizedSong, ShazamResponse};
use std::io::Cursor;

#[test]
fn test_signature_generation_on_synthetic_sine_wave() {
    let sample_rate = 16000;
    let duration_secs = 5;
    let num_samples = sample_rate * duration_secs;

    // Generate 440Hz (A4) + 880Hz (A5) chord in 16-bit PCM
    let pcm: Vec<i16> = (0..num_samples)
        .map(|i| {
            let f1 = (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin();
            let f2 = (i as f32 * 880.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin();
            ((f1 * 0.5 + f2 * 0.5) * 20000.0) as i16
        })
        .collect();

    let mut gen = SignatureGenerator::new();
    let uri = gen
        .generate_signature(&pcm)
        .expect("Signature generation failed");

    // Also test one-shot helper
    let uri2 = gen
        .generate_from_i16(&pcm)
        .expect("generate_from_i16 failed");
    assert_eq!(uri, uri2);

    // 1. Check URI prefix
    assert!(uri.starts_with(DATA_URI_PREFIX));

    // 2. Decode base64
    let b64_payload = &uri[DATA_URI_PREFIX.len()..];
    let binary = base64::Engine::decode(&base64::prelude::BASE64_STANDARD, b64_payload)
        .expect("Valid Base64 payload");

    assert!(
        binary.len() > 48,
        "Binary signature must have header > 48 bytes"
    );

    // 3. Verify magic bytes
    let mut cursor = Cursor::new(&binary);
    let magic1 = cursor.read_u32::<LittleEndian>().unwrap();
    assert_eq!(magic1, 0xcafe2580, "Magic 1 must match Shazam standard");

    let crc = cursor.read_u32::<LittleEndian>().unwrap();
    let size_minus_header = cursor.read_u32::<LittleEndian>().unwrap();
    assert_eq!(size_minus_header as usize, binary.len() - 48);

    let magic2 = cursor.read_u32::<LittleEndian>().unwrap();
    assert_eq!(magic2, 0x94119c00, "Magic 2 must match Shazam standard");

    // 4. Verify CRC32 checksum
    let mut hasher = Hasher::new();
    hasher.update(&binary[8..]);
    assert_eq!(crc, hasher.finalize(), "CRC-32 checksum must match payload");
}

#[test]
fn test_buffer_too_short_fails_gracefully() {
    let pcm = vec![0i16; 100]; // Less than 128 * 46
    let res = SignatureGenerator::make_signature(&pcm);
    assert!(res.is_err(), "Too short buffer must return error");
}

#[test]
fn test_model_from_shazam_response() {
    let json_data = serde_json::json!({
        "matches": [{
            "id": "123456",
            "offset": 42.5
        }],
        "track": {
            "key": "601510697",
            "title": "Aankhon Se Batana",
            "subtitle": "Dikshant",
            "isrc": "TCAGA2213003",
            "images": {
                "coverart": "https://example.com/cover.jpg",
                "coverarthq": "https://example.com/cover_hq.jpg"
            },
            "genres": {
                "primary": "Indian Pop"
            },
            "share": {
                "href": "https://www.shazam.com/track/601510697/aankhon-se-batana"
            }
        }
    });

    let resp: ShazamResponse = serde_json::from_value(json_data).expect("Deserialization");
    let song = RecognizedSong::from_shazam_response(resp).expect("Parsed song");

    assert_eq!(song.title, "Aankhon Se Batana");
    assert_eq!(song.artist, "Dikshant");
    assert_eq!(song.isrc.as_deref(), Some("TCAGA2213003"));
    assert_eq!(song.genre.as_deref(), Some("Indian Pop"));
    assert_eq!(song.offset_seconds, Some(42.5));
    assert_eq!(song.display_id(), "Aankhon Se Batana - Dikshant");
}
