# 🎵 shazam-daemon

A high-performance, real-time audio recognition daemon and native D-Bus MPRIS2 service written in Rust.

Performs continuous, low-latency song identification directly from desktop audio output or microphone streams, using a Rust port of the Shazam landmark fingerprinting that [SongRec](https://github.com/marin-m/SongRec) reverse-engineered.

---

## ✨ Features

- **⚡ Native Rust DSP Pipeline**: SIMD-accelerated Fast Fourier Transform (`realfft` / `rustfft`), 2048-point Hanning windowing with 128-sample stride.
- **🎯 Exact Shazam Landmark Binary Serialization**: Direct encoding into Shazam's `0xcafe2580` / `0x94119c00` binary signature format with CRC32 checksums.
- **🔇 RMS Silence Gating**: Energy thresholding (`-45 dBFS`) drops ambient silence before FFT and network calls, saving CPU and bandwidth.
- **🔄 Lock-Free Ring Buffering**: Single-Producer Single-Consumer (`rtrb`) audio capture ensures audio streams never block or drop frames.
- **📻 PipeWire / PulseAudio Auto-Capture**: Automatically resolves default output sink monitor (desktop playback) and falls back to microphone input.
- **📡 Deep Cloud Metadata Extraction**:
  - Track Title & Artist
  - Album & Genre
  - **ISRC (International Standard Recording Code)**
  - **Global Shazam Catalog Key**
  - **Track Time Offset** (seconds within original recording)
  - **Apple Music 30s AAC Audio Preview Stream URL**
  - **Synchronized Lyrics**
- **🎛️ Native D-Bus MPRIS2 Server (`org.mpris.MediaPlayer2.Shazam`)**:
  - Automatically picked up by Quickshell, Waybar, KDE, GNOME, and `playerctl`.
  - Zero-polling event delivery via D-Bus property change signals.
- **📜 Dual History Logging**: Writes human-readable text (`~/.local/share/shazam_history.txt`) and structured JSON lines (`~/.local/share/shazam_history.jsonl`).

---

## 🚀 Installation & Build

```bash
git clone https://github.com/omsingh02/shazam-daemon.git
cd shazam-daemon
cargo build --release

# Install to user bin
cp target/release/shazam-daemon ~/.local/bin/
```

---

## 🛠️ Usage

### Run as background daemon
```bash
shazam-daemon --waybar
```

### Toggle listening state (Active / Paused)
```bash
shazam-daemon --toggle
```

### Control via playerctl
```bash
# Check status
playerctl -p Shazam status

# Toggle listening
playerctl -p Shazam play-pause

# View metadata of current identified track
playerctl -p Shazam metadata
```

### Query status
```bash
shazam-daemon --status
```

---

## 🧩 Waybar Integration

Add the custom module to your Waybar `config.jsonc`:

```jsonc
"custom/shazam": {
    "format": "{}",
    "return-type": "json",
    "exec": "~/.local/bin/musicRecognition/shazam-waybar.sh",
    "restart-interval": 5,
    "on-click": "shazam-daemon --toggle",
    "on-click-right": "foot --title 'Shazam History' sh -c 'bat ~/.local/share/shazam_history.txt; read -p \"Press enter to close...\"'",
    "tooltip": true
}
```

---

## 🗂️ Project Layout

```
crates/shazam-core   landmark fingerprinting (FFT, signature encoding) and Shazam response models
crates/jiosaavn      JioSaavn search, streaming and tagging client
src/                 the daemon: audio capture, MPRIS2 service, history, cover-art cache, Shazam client
```

---

## 📜 License

GPL-3.0-or-later. See [LICENSE](LICENSE).

The fingerprinting code in `crates/shazam-core/src/dsp` is derived from [SongRec](https://github.com/marin-m/SongRec) by marin-m, which is licensed GPL-3.0-or-later.
