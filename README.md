<p align="center">
  <img src="assets/banner.svg" alt="hark - always-on song recognition for Linux" width="560">
</p>

<p align="center">
  <a href="https://github.com/omsingh02/hark/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/omsingh02/hark/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="License: GPL-3.0-or-later" src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue"></a>
  <img alt="Rust 1.88+" src="https://img.shields.io/badge/rust-1.88%2B-orange">
  <img alt="Platform: Linux" src="https://img.shields.io/badge/platform-linux-lightgrey">
</p>

**hark** is a small background service that listens for music and tells your desktop what is playing. It identifies songs continuously and publishes the result as a standard MPRIS2 media player, so Waybar, Quickshell, GNOME and KDE widgets, and `playerctl` can show the current track and cover art with no extra glue code.

```console
$ playerctl -p hark metadata --format '{{ artist }} - {{ title }}'
Daft Punk - Digital Love
```

> hark is pre-1.0. Command-line options and the D-Bus interface may still change.

## Features

- **Continuous recognition.** Keeps a 12-second rolling buffer, fingerprints it locally, and asks Shazam to identify it. It checks about every 3.5 seconds while searching and backs off to every 25 seconds once a song is confirmed.
- **A stable "now playing".** A different track must be recognized twice in a row before it replaces the current one. Remixes, alternate versions and the continuing playback of the same song are not treated as new tracks.
- **A real media player.** Registers `org.mpris.MediaPlayer2.hark` with title, artist, album, genre, cover art, playback position, ISRC, lyrics and share links. Play and Pause resume and pause listening.
- **Quiet by design.** Nothing is sent while the input is silent, and the audio device is released while hark is paused.
- **Searchable history.** Every identified song is saved locally, de-duplicated, and can be searched or deleted over D-Bus.
- **Cover-art cache.** Artwork is cached on disk (capped at 100 MiB), so bars show it instantly.
- **Bar integration.** Writes a JSON state file for Waybar. Any other bar or widget that understands MPRIS works without configuration.
- **Light on resources.** Fingerprinting a 12-second window takes roughly 13 ms on a recent Ryzen CPU (reproduce with `cargo run --release --example benchmark`).
- **One native binary.** Written in Rust with TLS provided by rustls. No Python or Node runtime.

## How it works

```
input device -> 16 kHz mono -> 12 s ring buffer -> silence gate -> fingerprint -> Shazam
 (ALSA)         resampler                          (-55 dBFS)      (hark-core)      |
                                                                                    v
playerctl, bars  <-  MPRIS2 on D-Bus  <-  track tracker (confirm twice)  <-  best match
Waybar           <-  runtime state files        history + cover cache  <---------+
```

The fingerprint is computed on your machine. Only the fingerprint is sent, never audio. See [Privacy](#privacy).

## Requirements

- Linux with a D-Bus session bus (any desktop or compositor).
- A sound server that exposes an ALSA input: PipeWire (with `pipewire-alsa`) or PulseAudio.
- Network access.
- To build: Rust 1.88 or newer, `pkg-config` and the ALSA development headers.

| Distribution | Packages |
| --- | --- |
| Arch | `pacman -S rust alsa-lib pipewire-alsa` |
| Debian, Ubuntu | `apt install build-essential pkg-config libasound2-dev` (Rust via [rustup](https://rustup.rs)) |
| Fedora | `dnf install gcc pkgconf-pkg-config alsa-lib-devel rust cargo` |

## Installation

```sh
cargo install --git https://github.com/omsingh02/hark --locked
```

This puts `hark` in `~/.cargo/bin`. To build from a checkout instead:

```sh
git clone https://github.com/omsingh02/hark.git
cd hark
cargo build --release --locked
install -Dm755 target/release/hark ~/.local/bin/hark
```

## Quick start

Start the daemon in a terminal to see it working:

```sh
hark
```

In another terminal, once a song has been identified:

```sh
playerctl -p hark metadata            # everything hark knows about the track
playerctl -p hark metadata --format '{{ artist }} - {{ title }}'
hark --toggle                         # pause or resume listening
hark --status                         # {"running":true}
```

### Start at login

A systemd user unit is provided in [`contrib/systemd/hark.service`](contrib/systemd/hark.service):

```sh
install -Dm644 contrib/systemd/hark.service ~/.config/systemd/user/hark.service
systemctl --user daemon-reload
systemctl --user enable --now hark
journalctl --user -u hark -f          # follow the log
```

The unit expects the binary at `~/.cargo/bin/hark`. If you installed it elsewhere, run `systemctl --user edit --full hark` and change `ExecStart`.

## Integrations

### Waybar

hark keeps `$XDG_RUNTIME_DIR/hark/waybar.json` up to date. [`contrib/waybar/`](contrib/waybar) contains a small wrapper script, a module definition and CSS:

```sh
install -Dm755 contrib/waybar/hark-waybar ~/.local/bin/hark-waybar
```

Then merge `config.jsonc` into your Waybar config and `style.css` into your stylesheet; the comments at the top of each file explain the steps. The icons are Nerd Font glyphs, so a [Nerd Font](https://www.nerdfonts.com) must be installed.

The module's CSS class tells you what hark is doing:

| Class | Meaning |
| --- | --- |
| `ambient` | Listening, nothing identified yet |
| `found` | A song is identified; the text shows its title |
| `paused` | Listening is paused |
| `offline` | The network is unreachable |
| `ratelimited` | Shazam is throttling requests; hark is backing off |

### Hyprland

[`contrib/hyprland/hark.conf`](contrib/hyprland/hark.conf) has example key bindings for toggling listening and showing the current track.

### Other bars and widgets

hark appears as an ordinary MPRIS player named `hark`. If a media widget switches to it when you would rather see your music player, filter it out (for example `playerctl -i hark`) or give your music player priority.

## Command line

| Option | Description |
| --- | --- |
| *(none)* | Start the daemon and run in the foreground. Only one instance can run at a time. |
| `--toggle` | Pause or resume listening on the running daemon. |
| `--status` | Print `{"running":true}` or `{"running":false}`. |
| `--source <auto\|mic\|monitor>` | Which input to listen to. `auto` and `mic` use the default input device. `monitor` prefers an input whose name contains "monitor", and falls back to the default input. |
| `-V`, `--version` | Print the version. |
| `-h`, `--help` | Print help. |

## D-Bus interface

hark owns `org.mpris.MediaPlayer2.hark` on the session bus, at `/org/mpris/MediaPlayer2`. It implements the standard `org.mpris.MediaPlayer2` and `org.mpris.MediaPlayer2.Player` interfaces.

**Player properties**

| Property | Value |
| --- | --- |
| `PlaybackStatus` | `Playing` while listening, `Paused` while paused |
| `EngineStatus` | `ambient` (listening, nothing identified), `found`, or `paused` |
| `Metadata` | See below |
| `Position` | Microseconds into the identified song, following the detected offset |
| `CanControl`, `CanPlay`, `CanPause` | `true`. `CanSeek`, `CanGoNext` and `CanGoPrevious` are `false`. |

**Metadata** contains the standard `mpris:trackid`, `mpris:artUrl`, `xesam:title`, `xesam:artist`, `xesam:album` and `xesam:genre`, plus these extras: `hark:engineStatus`, `hark:isrc`, `hark:offset` (seconds), `hark:youtubeUrl`, `hark:shareUrl` and `hark:lyrics` (newline-separated). `mpris:artUrl` is a `file://` URL once the cover is cached.

**Methods** `Play`, `Pause`, `PlayPause` and `Stop` resume or pause listening. hark adds:

| Method | Returns |
| --- | --- |
| `GetRecentHistory(u limit)` | JSON array of the most recent songs, newest first |
| `SearchHistory(s query)` | JSON array of matching songs |
| `DeleteHistoryEntry(s key_or_title, s artist)` | `true` if an entry was removed |
| `ClearHistory()` | `true` |
| `SetForeground(b in_foreground)` | `true`. Tell hark a UI is visible so it polls at the fast rate. |

```sh
busctl --user call org.mpris.MediaPlayer2.hark /org/mpris/MediaPlayer2 \
  org.mpris.MediaPlayer2.Player GetRecentHistory u 5
```

## Files

| Path | Contents |
| --- | --- |
| `$XDG_RUNTIME_DIR/hark/hark.pid` | PID of the running daemon |
| `$XDG_RUNTIME_DIR/hark/waybar.json` | `{"text", "tooltip", "class"}` for bars |
| `$XDG_RUNTIME_DIR/hark/state` | `active`, `paused`, `offline` or `ratelimited` |
| `$XDG_RUNTIME_DIR/hark/current` | `Title - Artist` while a song is identified |
| `~/.local/share/hark/history.jsonl` | History, one JSON record per line |
| `~/.local/share/hark/history.txt` | History as `[timestamp] Artist - Title` lines |
| `~/.cache/hark/covers/` | Cached cover art |

`XDG_DATA_HOME` and `XDG_CACHE_HOME` are respected. If you used shazam-daemon before, your old history is copied to the new location the first time hark runs; the original files are left untouched.

## Privacy

hark never uploads audio. For each recognition attempt it sends Shazam:

- the audio fingerprint (a compact list of spectral peaks, not recoverable as sound) and its duration;
- the current time;
- a placeholder location and time zone picked at random from a short built-in list. Your real location is not used.

As with any HTTPS request, Shazam's servers also see your IP address. Cover art is fetched from the URLs Shazam returns. Nothing is sent while the input is below the silence threshold or while hark is paused. Your history and cache stay on your machine.

## Troubleshooting

**`hark is not running`.** `--toggle` and `--status` talk to a running daemon. Start `hark`, or the systemd unit, first.

**Nothing is ever identified.** hark listens to the default input device, which is usually your microphone. Check that the level moves in your sound settings (`wpctl status` or `pavucontrol`), play the music louder or closer, and watch the output of `hark` in a terminal.

**`Another instance of hark is already running`.** Stop the other instance, or the systemd unit. If the process is gone but the message remains, delete `$XDG_RUNTIME_DIR/hark/hark.pid`.

**The `ratelimited` state.** Shazam throttles clients that ask too often. hark backs off by itself; pausing it when you are not listening to music helps.

**Boxes instead of icons in Waybar.** Install a Nerd Font and make sure Waybar uses it.

**ALSA or JACK messages at startup** (for example "jack server is not running"). These come from the audio plugins on your system and are harmless.

## Limitations

- Linux only.
- hark listens to the default input device. Capturing what your computer is playing (loopback of the speakers) is not built in yet. Making a monitor source your default input may work, but is untested. `--source monitor` only helps when your sound server shows an input named "monitor" to ALSA, which PipeWire and PulseAudio usually do not.
- Identification depends on Shazam's undocumented web API. It can change, throttle or stop working without notice.
- It recognizes recorded music. It will not identify humming or most live performances.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --example benchmark      # DSP and storage timings
cargo run --example test_capture   # live capture and API check (sends a fingerprint)
```

| Path | Contents |
| --- | --- |
| `src/` | The daemon: audio capture, MPRIS service, history, cover-art cache, Shazam client |
| `crates/hark-core` | Landmark fingerprinting and Shazam response models |
| `crates/jiosaavn` | Optional helper for fetching full tracks; not used for recognition |
| `contrib/` | systemd unit, Waybar and Hyprland examples |
| `examples/` | Benchmark and live capture check |

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). Notable changes are listed in [CHANGELOG.md](CHANGELOG.md), including how to move over from shazam-daemon, the previous name of this project.

## License

hark is licensed under the [GNU General Public License v3.0 or later](LICENSE).

## Acknowledgements

- [SongRec](https://github.com/marin-m/SongRec) by marin-m reverse-engineered the Shazam signature format. The fingerprinting code in `crates/hark-core/src/dsp` is derived from it, which is why hark is GPL-licensed.
- SongRec, [ShazamIO](https://github.com/shazamio/ShazamIO) and [Audile](https://github.com/AudileTeam/Audile) document the request format hark follows.
- Avery Li-Chun Wang, *An Industrial-Strength Audio Search Algorithm* (ISMIR 2003), describes the landmark fingerprinting approach.

## Disclaimer

hark is an independent project. It is not affiliated with, endorsed by or sponsored by Shazam or Apple Inc. "Shazam" is a trademark of Apple Inc. hark uses Shazam's undocumented web API, and its terms may restrict this kind of use. Use hark for personal purposes and at your own risk.
