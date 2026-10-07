mod audio;
mod cache;
mod downloader;
mod dsp;
mod history;
mod mpris;
mod network;

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::RwLock;
use zbus::connection::Builder;

use crate::audio::{AudioCapture, AudioSourceMode, SilenceDetector};
use crate::cache::CoverCacheManager;
use crate::downloader::JioSaavnDownloader;
use crate::dsp::SignatureGenerator;
use crate::history::{extract_base_title, extract_lead_artist, HistoryStorage};
use crate::mpris::{HarkPlayer, HarkRoot};
use crate::network::{RecognizedSong, ShazamClient};

/// Resolves the standard Freedesktop runtime directory: $XDG_RUNTIME_DIR/hark (Rule H-2)
fn runtime_dir() -> std::path::PathBuf {
    let dir = dirs::runtime_dir()
        .unwrap_or_else(|| {
            let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(base)
        })
        .join("hark");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn pid_file() -> std::path::PathBuf {
    runtime_dir().join("hark.pid")
}

fn state_file() -> std::path::PathBuf {
    runtime_dir().join("state")
}

fn current_file() -> std::path::PathBuf {
    runtime_dir().join("current")
}

fn json_file() -> std::path::PathBuf {
    runtime_dir().join("waybar.json")
}

#[derive(Parser, Debug)]
#[command(
    name = "hark",
    version,
    about = "Always-on song recognition for Linux, exposed as an MPRIS2 media player",
    after_help = "With no options, hark starts the daemon and keeps running in the foreground.\n\
                  Control a running daemon with --toggle and --status, or over D-Bus\n\
                  (org.mpris.MediaPlayer2.hark), for example: playerctl -p hark metadata"
)]
struct Cli {
    /// Deprecated no-op, kept so existing service files keep working
    #[arg(long, hide = true)]
    waybar: bool,

    #[arg(long, help = "Pause or resume listening on the running daemon")]
    toggle: bool,

    #[arg(long, help = "Print whether the daemon is running, as JSON")]
    status: bool,

    #[arg(
        long,
        help = "Audio input to listen to: auto and mic use the default input device, monitor prefers an input named \"monitor\"",
        default_value = "auto",
        value_parser = ["auto", "mic", "monitor"]
    )]
    source: String,

    #[arg(long, help = "Download a song by title and artist directly from JioSaavn 320kbps", num_args = 2, value_names = ["TITLE", "ARTIST"])]
    download: Option<Vec<String>>,

    #[arg(long, help = "Download the currently recognized song from running daemon")]
    download_current: bool,

    #[arg(long, help = "Stream full-length track directly from JioSaavn via mpv", num_args = 2, value_names = ["TITLE", "ARTIST"])]
    stream: Option<Vec<String>>,

    #[arg(long, help = "Play full track via system player (mpv) with complete MPRIS metadata and controls", num_args = 2, value_names = ["TITLE", "ARTIST"])]
    play: Option<Vec<String>>,
}

fn read_pid() -> Option<i32> {
    let path = pid_file();
    if path.exists() {
        let content = fs::read_to_string(&path).ok()?;
        if let Ok(pid) = content.trim().parse::<i32>() {
            // Rule U-3: Verify the process is hark by the program name in /proc/<pid>/cmdline
            let cmdline_path = format!("/proc/{}/cmdline", pid);
            if let Ok(cmdline) = fs::read_to_string(&cmdline_path) {
                let program = cmdline.split('\0').next().unwrap_or("");
                if std::path::Path::new(program).file_name().and_then(|n| n.to_str()) == Some("hark") {
                    return Some(pid);
                }
            }
        }
    }
    None
}

fn write_pid() {
    let pid = std::process::id();
    let _ = fs::write(pid_file(), pid.to_string());
}

fn cleanup_files() {
    let _ = fs::remove_file(pid_file());
    let _ = fs::remove_file(current_file());
    let _ = fs::remove_file(state_file());
    let _ = fs::remove_file(json_file());
}

fn emit_waybar_state(text: &str, tooltip: &str, class: &str) {
    let data = serde_json::json!({
        "text": text,
        "tooltip": tooltip,
        "class": class
    });
    let _ = fs::write(json_file(), data.to_string());
}

fn emit_paused() {
    let _ = fs::write(state_file(), "paused");
    let _ = fs::remove_file(current_file());
    emit_waybar_state("󰏤", "hark is paused. Click to listen.", "paused");
}

fn emit_listening() {
    let _ = fs::write(state_file(), "active");
    emit_waybar_state("󰓅", "hark is listening...", "ambient");
}

fn emit_offline() {
    let _ = fs::write(state_file(), "offline");
    emit_waybar_state("󰖪", "hark: network unreachable", "offline");
}

fn emit_ratelimited() {
    let _ = fs::write(state_file(), "ratelimited");
    emit_waybar_state("󱎫", "hark: rate-limited by Shazam, backing off", "ratelimited");
}

fn emit_found(song: &RecognizedSong) {
    let _ = fs::write(state_file(), "active");
    let clean_title = song.title.split('(').next().unwrap_or(&song.title).trim();
    let text = format!("󰓅 {}", clean_title);

    let mut tooltip = format!(
        "<span size='13000' weight='bold'>{}</span>\n<span size='11000' color='#cccccc'><i>by</i> {}</span>",
        html_escape(&song.title),
        html_escape(&song.artist)
    );
    if let Some(album) = &song.album {
        tooltip.push_str(&format!("\n<b>Album:</b> {}", html_escape(album)));
    }
    if let Some(genre) = &song.genre {
        tooltip.push_str(&format!("\n<b>Genre:</b> {}", html_escape(genre)));
    }

    emit_waybar_state(&text, &tooltip, "found");
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn check_is_foreground(is_fg_atomic: &AtomicBool) -> bool {
    is_fg_atomic.load(Ordering::SeqCst)
}

fn is_continuous_or_variant(
    active: &RecognizedSong,
    candidate: &RecognizedSong,
    last_detect_instant: Option<std::time::Instant>,
) -> bool {
    // 1. Exact track ID or ISRC
    if active.shazam_key.is_some() && active.shazam_key == candidate.shazam_key {
        return true;
    }
    if active.isrc.is_some() && active.isrc == candidate.isrc {
        return true;
    }

    // 2. Canonical base title and lead artist match (remix/mashup/alternate version)
    let active_base = extract_base_title(&active.title);
    let cand_base = extract_base_title(&candidate.title);
    let active_lead = extract_lead_artist(&active.artist);
    let cand_lead = extract_lead_artist(&candidate.artist);

    let same_family = !active_base.is_empty()
        && active_base == cand_base
        && !active_lead.is_empty()
        && active_lead == cand_lead;

    if same_family {
        return true;
    }

    // 3. Time-offset coherence (Wang 2003 continuous landmark tracking):
    // If the audio timeline progressed continuously and lead artists match, it is the same audio stream
    if let (Some(t_prev), Some(off_prev), Some(off_curr)) = (
        last_detect_instant,
        active.offset_seconds,
        candidate.offset_seconds,
    ) {
        let delta_wall = t_prev.elapsed().as_secs_f64();
        if delta_wall < 30.0 {
            let expected_offset = off_prev + delta_wall;
            let offset_diff = (off_curr - expected_offset).abs();
            if offset_diff <= 4.0 && !active_lead.is_empty() && active_lead == cand_lead {
                return true;
            }
        }
    }

    false
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    if let Some(args) = cli.download {
        let title = &args[0];
        let artist = &args[1];
        let client = JioSaavnDownloader::new();
        let download_dir = JioSaavnDownloader::get_music_dir();

        println!("Downloading: {} - {} (320kbps AAC)...", title, artist);
        let song = client.find_best_match(title, artist).await.map_err(|e| { eprintln!("Search error: {}", e); std::process::exit(1); }).unwrap();
        match client.download_song(&song, &download_dir).await {
            Ok(p) => {
                println!("Saved to: {}", p.display());
                return Ok(());
            }
            Err(e) => {
                eprintln!("Download error: {}", e);
                std::process::exit(1);
            }
        }
    }

    if cli.download_current {
        let current_text = fs::read_to_string(current_file()).unwrap_or_default();
        if let Some((title, artist)) = current_text.split_once(" - ") {
            let client = JioSaavnDownloader::new();
            let download_dir = JioSaavnDownloader::get_music_dir();

            println!("Downloading current track: {} - {} (320kbps AAC)...", title.trim(), artist.trim());
            let song = client.find_best_match(title.trim(), artist.trim()).await.map_err(|e| { eprintln!("Search error: {}", e); std::process::exit(1); }).unwrap();
            match client.download_song(&song, &download_dir).await {
                Ok(p) => {
                    println!("Saved to: {}", p.display());
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("Download error: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            eprintln!("No song is currently recognized.");
            std::process::exit(1);
        }
    }

    if let Some(args) = cli.stream {
        let title = &args[0];
        let artist = &args[1];
        let client = JioSaavnDownloader::new();
        let song = client.find_best_match(title, artist).await.map_err(|e| { eprintln!("Search error: {}", e); std::process::exit(1); }).unwrap();
        match client.get_stream_url(&song).await {
            Ok(info) => {
                let status = std::process::Command::new("mpv")
                    .arg("--no-video")
                    .arg("--volume=85")
                    .arg(&info.url)
                    .status();
                if let Ok(s) = status {
                    std::process::exit(s.code().unwrap_or(0));
                }
            }
            Err(e) => {
                eprintln!("Streaming error: {}", e);
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    if let Some(args) = cli.play {
        let title = &args[0];
        let artist = &args[1];
        let client = JioSaavnDownloader::new();
        let download_dir = JioSaavnDownloader::get_music_dir();

        match client.ensure_song(title, artist, &download_dir).await {
            Ok(path) => {
                match client.play_in_default_player(&path).await {
                    Ok(msg) => println!("{}", msg),
                    Err(e) => eprintln!("Playback error: {}", e),
                }
                return Ok(());
            }
            Err(e) => {
                eprintln!("Playback preparation error: {}", e);
                std::process::exit(1);
            }
        }
    }

    if cli.toggle {
        if let Some(pid) = read_pid() {
            unsafe {
                libc::kill(pid, libc::SIGUSR1);
            }
            println!("Toggled daemon (PID {})", pid);
        } else {
            eprintln!("hark is not running");
            std::process::exit(1);
        }
        return Ok(());
    }

    if cli.status {
        let running = read_pid().is_some();
        println!("{}", serde_json::json!({ "running": running }));
        return Ok(());
    }

    // Check single-instance
    if let Some(existing_pid) = read_pid() {
        if existing_pid != std::process::id() as i32 {
            eprintln!("Another instance of hark is already running (PID {})", existing_pid);
            std::process::exit(1);
        }
    }

    write_pid();

    let source_mode = match cli.source.to_lowercase().as_str() {
        "monitor" => AudioSourceMode::Monitor,
        "mic" => AudioSourceMode::Mic,
        _ => AudioSourceMode::Auto,
    };

    let audio_capture = AudioCapture::new(source_mode)?;
    let sig_gen = SignatureGenerator::new();
    let shazam_client = ShazamClient::new();
    let history_storage = Arc::new(HistoryStorage::new());
    let silence_detector = SilenceDetector::default();

    let is_listening = Arc::new(AtomicBool::new(true)); // Start active listening
    let engine_status = Arc::new(RwLock::new("ambient".to_string()));
    let current_song = Arc::new(RwLock::new(None::<RecognizedSong>));
    let is_foreground = Arc::new(AtomicBool::new(false));
    let foreground_notify = Arc::new(tokio::sync::Notify::new());
    let playback_anchor: Arc<RwLock<Option<(f64, std::time::Instant)>>> = Arc::new(RwLock::new(None));
    let cover_cache = Arc::new(CoverCacheManager::default_manager()?);
    let http_client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

    // Register D-Bus MPRIS server
    let player_service = HarkPlayer::new(
        is_listening.clone(),
        engine_status.clone(),
        current_song.clone(),
        is_foreground.clone(),
        foreground_notify.clone(),
        playback_anchor.clone(),
        history_storage.clone(),
        cover_cache.clone(),
    );
    let dbus_conn = Builder::session()?
        .name("org.mpris.MediaPlayer2.hark")?
        .serve_at("/org/mpris/MediaPlayer2", HarkRoot)?
        .serve_at("/org/mpris/MediaPlayer2", player_service)?
        .build()
        .await?;

    // Low-priority background warming of recent history cover art
    {
        let cache_clone = cover_cache.clone();
        let history_clone = history_storage.clone();
        let client_clone = http_client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let recent = history_clone.get_recent(30);
            for item in recent {
                let key = item.get("shazam_key").and_then(|v| v.as_str()).unwrap_or("").trim();
                let url = item.get("cover_art").and_then(|v| v.as_str()).unwrap_or("").trim();
                let is_valid_url = url.starts_with("http://") || url.starts_with("https://");
                if !key.is_empty() && key != "0" && is_valid_url && cache_clone.get_local_path(key).is_none() {
                    let _ = cache_clone.ensure_cached(&client_clone, url, key).await;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
            }
        });
    }

    // Setup UNIX signals
    let mut sigusr1 = signal(SignalKind::user_defined1())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;

    emit_listening();

    const BASE_INTERVAL_MS: u64 = 3500;
    const BACKOFF_STEP_MS: u64 = 5000;
    const MAX_BACKOFF_MS: u64 = 25000;

    let mut current_interval_ms: u64 = BASE_INTERVAL_MS;
    let mut last_detected_id = String::new();
    let mut last_detection_instant: Option<std::time::Instant> = None;
    let mut pending_candidate: Option<(RecognizedSong, u8)> = None;
    let mut miss_count = 0;

    println!("hark {} started", env!("CARGO_PKG_VERSION"));

    loop {
        if check_is_foreground(&is_foreground) {
            current_interval_ms = BASE_INTERVAL_MS;
        }

        tokio::select! {
            _ = sighup.recv() => {
                // Ignore SIGHUP
            }
            _ = foreground_notify.notified() => {
                // Instantly reset timing when popup enters foreground
                current_interval_ms = BASE_INTERVAL_MS;
            }
            _ = sigusr1.recv() => {
                let current_state = is_listening.load(Ordering::Relaxed);
                let new_state = !current_state;
                is_listening.store(new_state, Ordering::Relaxed);

                current_interval_ms = BASE_INTERVAL_MS;
                *playback_anchor.write().await = None;

                if new_state {
                    audio_capture.clear_buffer();
                    last_detected_id.clear();
                    last_detection_instant = None;
                    pending_candidate = None;
                    audio_capture.resume();
                    *engine_status.write().await = "ambient".to_string();
                    emit_listening();
                } else {
                    audio_capture.pause();
                    audio_capture.clear_buffer();
                    last_detected_id.clear();
                    last_detection_instant = None;
                    pending_candidate = None;
                    *current_song.write().await = None;
                    *engine_status.write().await = "paused".to_string();
                    emit_paused();
                }

                if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                    let _ = iface_ref.get().await.playback_status_changed(iface_ref.signal_emitter()).await;
                    let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                    let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                }
            }
            _ = sigterm.recv() => {
                break;
            }
            _ = sigint.recv() => {
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(current_interval_ms)) => {
                if !is_listening.load(Ordering::Relaxed) {
                    continue;
                }

                // Require at least 3.0s in the ring buffer (48,000 samples)
                if audio_capture.sample_count() < 48000 {
                    continue;
                }

                // Extract the most recent 5-second chunk from the 12-second ring buffer
                let samples = audio_capture.extract_chunk(5);
                if samples.is_empty() {
                    continue;
                }

                // Energy Gating: Skip DSP if audio is silent
                let (is_silent, _dbfs) = silence_detector.is_silent(&samples);
                if is_silent {
                    if !last_detected_id.is_empty() {
                        miss_count += 1;
                        if miss_count >= 5 {
                            last_detected_id.clear();
                            last_detection_instant = None;
                            pending_candidate = None;
                            current_interval_ms = BASE_INTERVAL_MS;
                            *playback_anchor.write().await = None;
                            *current_song.write().await = None;
                            *engine_status.write().await = "ambient".to_string();
                            let _ = fs::remove_file(current_file());
                            emit_listening();

                            if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                                let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                                let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                                let _ = iface_ref.get().await.playback_status_changed(iface_ref.signal_emitter()).await;
                            }
                        }
                    }
                    continue;
                }

                // Generate signature from contiguous PCM samples
                let Some(sig_uri) = sig_gen.generate_from_i16(&samples) else {
                    continue;
                };

                let sample_ms = (samples.len() as f32 / 16.0) as u32;

                // Query Cloud API
                match shazam_client.recognize(&sig_uri, sample_ms).await {
                    Ok(Some(song)) => {
                        miss_count = 0;
                        let now = std::time::Instant::now();
                        let current_opt = current_song.read().await.clone();

                        if let Some(active) = current_opt {
                            // CASE 1: A song is already in Now Playing
                            if is_continuous_or_variant(&active, &song, last_detection_instant) {
                                // Same song, remix variant, or continuous audio stream!
                                if let Some(off) = song.offset_seconds {
                                    *playback_anchor.write().await = Some((off, now));
                                }
                                last_detection_instant = Some(now);
                                pending_candidate = None;

                                let is_fg = check_is_foreground(&is_foreground);
                                if is_fg {
                                    // Foreground: incremental timing is STOPPED and RESET!
                                    current_interval_ms = BASE_INTERVAL_MS;
                                } else {
                                    // Background: increment request delay step-by-step
                                    current_interval_ms = (current_interval_ms + BACKOFF_STEP_MS).min(MAX_BACKOFF_MS);
                                }
                                continue;
                            }

                            // CASE 2: Different track candidate detected
                            // Require at least 2 consecutive confirmations before switching Now Playing
                            let mut should_switch = false;
                            if let Some((ref cand, count)) = pending_candidate {
                                if is_continuous_or_variant(cand, &song, last_detection_instant) {
                                    if count + 1 >= 2 {
                                        should_switch = true;
                                    } else {
                                        pending_candidate = Some((song.clone(), count + 1));
                                    }
                                } else {
                                    pending_candidate = Some((song.clone(), 1));
                                }
                            } else {
                                pending_candidate = Some((song.clone(), 1));
                            }

                            if !should_switch {
                                // Still evaluating candidate; hold current song and continue at base speed
                                current_interval_ms = BASE_INTERVAL_MS;
                                continue;
                            }
                        }

                        // Confirmed new song detection (either from ambient or confirmed 2x candidate)
                        if let Some(off) = song.offset_seconds {
                            *playback_anchor.write().await = Some((off, now));
                        }
                        current_interval_ms = BASE_INTERVAL_MS;
                        last_detected_id = song.display_id();
                        last_detection_instant = Some(now);
                        pending_candidate = None;
                        history_storage.log_song(&song);
                        emit_found(&song);
                        let _ = fs::write(current_file(), format!("{} - {}", song.title, song.artist));
                        *current_song.write().await = Some(song.clone());
                        *engine_status.write().await = "found".to_string();

                        if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                            let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                            let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                            let _ = iface_ref.get().await.playback_status_changed(iface_ref.signal_emitter()).await;
                        }

                        // Background cover art caching (detached, non-blocking)
                        let cover_url_opt = song.cover_art_hq_url.clone().or_else(|| song.cover_art_url.clone());
                        if let (Some(cover_url), Some(key)) = (cover_url_opt, song.shazam_key.clone()) {
                            let cache_clone = cover_cache.clone();
                            let client_clone = http_client.clone();
                            let dbus_clone = dbus_conn.clone();
                            tokio::spawn(async move {
                                match cache_clone.ensure_cached(&client_clone, &cover_url, &key).await {
                                    Ok(_path) => {
                                        if let Ok(iface_ref) = dbus_clone.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                                            let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("[CoverCache] Failed to cache cover art for {}: {}", key, e);
                                    }
                                }
                            });
                        }
                    }
                    Ok(None) => {
                        miss_count += 1;
                        if miss_count >= 5 && !last_detected_id.is_empty() {
                            last_detected_id.clear();
                            last_detection_instant = None;
                            pending_candidate = None;
                            current_interval_ms = BASE_INTERVAL_MS;
                            *playback_anchor.write().await = None;
                            *current_song.write().await = None;
                            *engine_status.write().await = "ambient".to_string();
                            let _ = fs::remove_file(current_file());
                            emit_listening();

                            if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                                let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                                let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                                let _ = iface_ref.get().await.playback_status_changed(iface_ref.signal_emitter()).await;
                            }
                        } else if last_detected_id.is_empty() {
                            let was_offline = {
                                let mut status_lock = engine_status.write().await;
                                if *status_lock == "offline" || *status_lock == "ratelimited" {
                                    *status_lock = "ambient".to_string();
                                    true
                                } else {
                                    false
                                }
                            };
                            if was_offline {
                                emit_listening();
                                if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                                    let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        let is_ratelimit = err_msg.contains("429") || err_msg.contains("Rate Limited");
                        eprintln!("Recognition request failed: {}", err_msg);

                        miss_count += 1;

                        if miss_count >= 5 {
                            let new_status = if is_ratelimit { "ratelimited" } else { "offline" };
                            if !last_detected_id.is_empty() {
                                last_detected_id.clear();
                                last_detection_instant = None;
                                pending_candidate = None;
                                current_interval_ms = BASE_INTERVAL_MS;
                                *playback_anchor.write().await = None;
                                *current_song.write().await = None;
                                let _ = fs::remove_file(current_file());
                            }

                            let became_new_status = {
                                let mut status_lock = engine_status.write().await;
                                if *status_lock != new_status {
                                    *status_lock = new_status.to_string();
                                    true
                                } else {
                                    false
                                }
                            };
                            if became_new_status {
                                if is_ratelimit {
                                    emit_ratelimited();
                                } else {
                                    emit_offline();
                                }
                                if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                                    let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                                    let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                                }
                            }

                            let backoff_secs = if is_ratelimit { 45 } else { 15 };
                            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;

                            // After backoff cooldown, auto-resume ambient listening
                            {
                                let mut status_lock = engine_status.write().await;
                                if *status_lock == new_status {
                                    *status_lock = "ambient".to_string();
                                }
                            }
                            emit_listening();
                            if let Ok(iface_ref) = dbus_conn.object_server().interface::<_, HarkPlayer>("/org/mpris/MediaPlayer2").await {
                                let _ = iface_ref.get().await.engine_status_changed(iface_ref.signal_emitter()).await;
                                let _ = iface_ref.get().await.metadata_changed(iface_ref.signal_emitter()).await;
                            }
                        } else {
                            // Transient failure: backoff briefly and keep the current song in Now Playing
                            let backoff = if is_ratelimit { Duration::from_secs(5) } else { Duration::from_secs(2) };
                            tokio::time::sleep(backoff).await;
                        }
                    }
                }
            }
        }
    }

    cleanup_files();
    Ok(())
}
