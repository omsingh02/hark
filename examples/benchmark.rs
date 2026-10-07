use hark::audio::resampler::AudioResampler;
use hark::audio::silence::SilenceDetector;
use hark::dsp::SignatureGenerator;
use hark::history::HistoryStorage;
use hark::network::models::RecognizedSong;
use std::time::{Duration, Instant};

fn format_duration(d: Duration) -> String {
    let micros = d.as_micros();
    if micros < 1000 {
        format!("{} µs", micros)
    } else if micros < 1_000_000 {
        format!("{:.2} ms", micros as f64 / 1000.0)
    } else {
        format!("{:.3} s", d.as_secs_f64())
    }
}

fn stats(durations: &mut [Duration]) -> (Duration, Duration, Duration, Duration) {
    durations.sort();
    let sum: Duration = durations.iter().copied().sum();
    let avg = sum / durations.len() as u32;
    let p50 = durations[durations.len() * 50 / 100];
    let p95 = durations[durations.len() * 95 / 100];
    let p99 = durations[durations.len() * 99 / 100];
    (avg, p50, p95, p99)
}

fn main() {
    println!("============================================================");
    println!("            HARK INTERNAL FEATURE BENCHMARKS            ");
    println!("============================================================\n");

    // ------------------------------------------------------------
    // 1. Audio Resampler Benchmark (44.1 kHz & 48 kHz stereo -> 16 kHz mono)
    // ------------------------------------------------------------
    println!("--- 1. Audio Resampler (In-Memory DSP Conversion) ---");
    let test_seconds = 5;
    let stereo_44100_samples: Vec<f32> = (0..(44100 * 2 * test_seconds))
        .map(|i| ((i as f32) * 0.05).sin())
        .collect();
    let stereo_48000_samples: Vec<f32> = (0..(48000 * 2 * test_seconds))
        .map(|i| ((i as f32) * 0.05).sin())
        .collect();

    // Warmup
    let _ = AudioResampler::resample_to_16k_mono(&stereo_44100_samples, 2, 44100);

    let iterations = 100;
    let mut times_441 = Vec::with_capacity(iterations);
    let mut out_len_441 = 0;
    for _ in 0..iterations {
        let t0 = Instant::now();
        let res = AudioResampler::resample_to_16k_mono(&stereo_44100_samples, 2, 44100);
        times_441.push(t0.elapsed());
        out_len_441 = res.len();
    }
    let (avg_441, p50_441, p95_441, _) = stats(&mut times_441);
    let input_samples_441 = stereo_44100_samples.len();
    let throughput_441 = (input_samples_441 as f64 / avg_441.as_secs_f64()) / 1_000_000.0;
    let rtf_441 = test_seconds as f64 / avg_441.as_secs_f64();

    println!("  [44.1 kHz Stereo -> 16 kHz Mono (5.0s audio chunk)]");
    println!(
        "    * Output Samples:    {} (exact 16 kHz target)",
        out_len_441
    );
    println!("    * Latency (Mean):    {}", format_duration(avg_441));
    println!("    * Latency (p50):     {}", format_duration(p50_441));
    println!("    * Latency (p95):     {}", format_duration(p95_441));
    println!(
        "    * DSP Throughput:    {:.2} MSamples/sec",
        throughput_441
    );
    println!(
        "    * Realtime Speedup:  {:.1}x real-time (5s audio converted in {})",
        rtf_441,
        format_duration(avg_441)
    );

    let mut times_48 = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let t0 = Instant::now();
        let _ = AudioResampler::resample_to_16k_mono(&stereo_48000_samples, 2, 48000);
        times_48.push(t0.elapsed());
    }
    let (avg_48, p50_48, p95_48, _) = stats(&mut times_48);
    let throughput_48 = (stereo_48000_samples.len() as f64 / avg_48.as_secs_f64()) / 1_000_000.0;
    let rtf_48 = test_seconds as f64 / avg_48.as_secs_f64();

    println!("  [48.0 kHz Stereo -> 16 kHz Mono (5.0s audio chunk)]");
    println!("    * Latency (Mean):    {}", format_duration(avg_48));
    println!("    * Latency (p50):     {}", format_duration(p50_48));
    println!("    * Latency (p95):     {}", format_duration(p95_48));
    println!("    * DSP Throughput:    {:.2} MSamples/sec", throughput_48);
    println!("    * Realtime Speedup:  {:.1}x real-time\n", rtf_48);

    // ------------------------------------------------------------
    // 2. Silence Detector & Energy Gating Benchmark
    // ------------------------------------------------------------
    println!("--- 2. Silence Detector & Energy Gating ---");
    let silence = SilenceDetector::default();
    let pcm_5s: Vec<i16> = (0..(16000 * 5)).map(|i| (i % 32767) as i16).collect();
    let mut times_silence = Vec::with_capacity(500);

    for _ in 0..500 {
        let t0 = Instant::now();
        let _ = silence.is_silent(&pcm_5s);
        times_silence.push(t0.elapsed());
    }
    let (avg_sil, p50_sil, p95_sil, _) = stats(&mut times_silence);
    let sil_throughput = (pcm_5s.len() as f64 / avg_sil.as_secs_f64()) / 1_000_000.0;
    let sil_rtf = 5.0 / avg_sil.as_secs_f64();

    println!("  [dBFS Energy Gating on 80,000 samples (5.0s PCM)]");
    println!("    * Latency (Mean):    {}", format_duration(avg_sil));
    println!("    * Latency (p50):     {}", format_duration(p50_sil));
    println!("    * Latency (p95):     {}", format_duration(p95_sil));
    println!(
        "    * Gating Speed:      {:.2} MSamples/sec ({:.0}x real-time)\n",
        sil_throughput, sil_rtf
    );

    // ------------------------------------------------------------
    // 3. DSP Fingerprinting & Signature Generation
    // ------------------------------------------------------------
    println!(
        "--- 3. DSP Fingerprinting & Signature Generation (Native Rust FFT via hark-core) ---"
    );
    let sig_gen = SignatureGenerator::new();

    let durations_test = [3, 5, 8, 12];
    for &sec in &durations_test {
        let samples: Vec<i16> = (0..(16000 * sec))
            .map(|i| {
                let f1 = (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin();
                let f2 = (i as f32 * 880.0 * 2.0 * std::f32::consts::PI / 16000.0).sin();
                ((f1 * 0.5 + f2 * 0.5) * 20000.0) as i16
            })
            .collect();

        // Warmup
        let _ = sig_gen.generate_from_i16(&samples);

        let mut sig_times = Vec::with_capacity(30);
        let mut sig_len = 0;
        for _ in 0..30 {
            let t0 = Instant::now();
            if let Some(uri) = sig_gen.generate_from_i16(&samples) {
                sig_times.push(t0.elapsed());
                sig_len = uri.len();
            }
        }

        if !sig_times.is_empty() {
            let (avg_sig, p50_sig, p95_sig, _) = stats(&mut sig_times);
            let rtf_sig = sec as f64 / avg_sig.as_secs_f64();
            println!(
                "  [{}s Audio Sample Chunk ({} samples)]",
                sec,
                samples.len()
            );
            println!("    * Signature Size:    {} bytes (Base64 URI)", sig_len);
            println!("    * Latency (Mean):    {}", format_duration(avg_sig));
            println!("    * Latency (p50):     {}", format_duration(p50_sig));
            println!("    * Latency (p95):     {}", format_duration(p95_sig));
            println!(
                "    * DSP Speedup:       {:.1}x real-time (fingerprinted in {})",
                rtf_sig,
                format_duration(avg_sig)
            );
        }
    }
    println!();

    // ------------------------------------------------------------
    // 4. Ring Buffer Operations (Lock-Free Memory Management)
    // ------------------------------------------------------------
    println!("--- 4. Ring Buffer (Sample Storage & Window Extraction) ---");
    let mut ring = hark::audio::RingBuffer::new();
    let chunk_512: Vec<i16> = vec![1234; 512];
    let mut push_times = Vec::with_capacity(1000);

    for _ in 0..1000 {
        let t0 = Instant::now();
        ring.push_slice(&chunk_512);
        push_times.push(t0.elapsed());
    }
    let (avg_push, _p50_push, p95_push, _) = stats(&mut push_times);

    let mut extract_times = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let t0 = Instant::now();
        let _ = ring.extract_recent(16000 * 5); // 5s extraction
        extract_times.push(t0.elapsed());
    }
    let (avg_ext, _p50_ext, p95_ext, _) = stats(&mut extract_times);

    println!("  [Ring Buffer Operations (12s / 192,000 capacity)]");
    println!(
        "    * Push 512 Samples Latency:   Mean: {}, p95: {}",
        format_duration(avg_push),
        format_duration(p95_push)
    );
    println!(
        "    * Extract 5s (80k samples):   Mean: {}, p95: {}",
        format_duration(avg_ext),
        format_duration(p95_ext)
    );
    println!("    * Memory Overhead:            384.0 KB (Fixed contiguous ring)\n");

    // ------------------------------------------------------------
    // 5. History Storage Engine (JSONL Persistence & Search)
    // ------------------------------------------------------------
    println!("--- 5. History Storage Engine (JSONL Persistence) ---");
    let storage = HistoryStorage::new();
    let test_song = RecognizedSong {
        shazam_key: Some("12345678".into()),
        title: "Benchmark Track".into(),
        artist: "Benchmark Artist".into(),
        album: Some("Benchmark Album".into()),
        genre: Some("Electronic".into()),
        cover_art_url: Some("https://example.com/art.jpg".into()),
        cover_art_hq_url: Some("https://example.com/art_hq.jpg".into()),
        preview_audio_url: Some("https://example.com/audio.mp3".into()),
        youtube_url: Some("https://youtube.com/watch?v=123".into()),
        share_url: Some("https://shazam.com/track/123".into()),
        lyrics: Some(vec!["Line 1".into(), "Line 2".into()]),
        offset_seconds: Some(12.5),
        isrc: Some("USRC12345678".into()),
    };

    let mut write_times = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        storage.log_song(&test_song);
        write_times.push(t0.elapsed());
    }
    let (avg_w, _p50_w, p95_w, _) = stats(&mut write_times);

    let mut read_times = Vec::with_capacity(100);
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = storage.get_recent(50);
        read_times.push(t0.elapsed());
    }
    let (avg_r, _p50_r, p95_r, _) = stats(&mut read_times);

    println!("  [History Persistence (Append-Only JSONL)]");
    println!(
        "    * Atomic Log Song Latency:    Mean: {}, p95: {}",
        format_duration(avg_w),
        format_duration(p95_w)
    );
    println!(
        "    * Parse Last 50 Tracks:       Mean: {}, p95: {}\n",
        format_duration(avg_r),
        format_duration(p95_r)
    );

    println!("============================================================");
    println!("                  BENCHMARK RUN COMPLETED                   ");
    println!("============================================================");
}
