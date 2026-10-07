# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Renamed the project from `shazam-daemon` to `hark`. This breaks anything that talks to the daemon:
  - The binary is now `hark`.
  - The D-Bus name is now `org.mpris.MediaPlayer2.hark` (was `org.mpris.MediaPlayer2.Shazam`) and the MPRIS identity is now "Hark" (was "Shazam"). Use `playerctl -p hark`.
  - Custom metadata keys use the `hark:` prefix instead of `shazam:`: `hark:engineStatus`, `hark:isrc`, `hark:offset`, `hark:youtubeUrl`, `hark:shareUrl` and `hark:lyrics`.
  - Runtime files moved from `$XDG_RUNTIME_DIR/shazam-daemon/` to `$XDG_RUNTIME_DIR/hark/` and were renamed: `shazam-scanner.pid` is now `hark.pid`, `waybar-shazam-state` is now `state`, `waybar-shazam-current` is now `current` and `waybar-shazam-json` is now `waybar.json`.
  - History moved from `~/.local/share/shazam_history.txt` and `shazam_history.jsonl` to `~/.local/share/hark/history.txt` and `history.jsonl`. Existing history is copied on the first start if the new files do not exist. The old files are left in place.
  - The cover-art cache moved from `~/.cache/shazam/covers` to `~/.cache/hark/covers`.
- The fingerprinting code and response models live in the `hark-core` crate (formerly `shazam-core`).
- `--source` now rejects values other than `auto`, `mic` and `monitor` instead of silently using `auto`.
- `--waybar` is still accepted so existing service files keep working, but it does nothing and is hidden from `--help`.
- The project is licensed under GPL-3.0-or-later. The fingerprinting code is derived from SongRec.
- Recognition backs off while the same song keeps playing, needs two consecutive matches before switching tracks, and treats remixes and continuous audio as the same song.

### Added

- `hark --version`.
- The `benchmark` and `test_capture` programs are now examples (`cargo run --example benchmark`), so `cargo install` only installs `hark`.
- `DeleteHistoryEntry` method on the D-Bus Player interface.
- Continuous integration (format, clippy, tests and a release build) and a release workflow that publishes an x86_64 Linux tarball with a checksum when a version tag is pushed.
- Example integrations in `contrib/`: a systemd user unit, a Waybar module and Hyprland keybinds.

### Removed

- The `GetPreviewUrl` D-Bus method and the `shazam:previewUrl` metadata key.
- The `--download`, `--download-current`, `--stream` and `--play` options, the `DownloadCurrent`, `DownloadTrack` and `GetStreamUrl` D-Bus methods, and the `jiosaavn` crate. hark only recognizes music; fetching full tracks is out of scope.

### Fixed

- The `benchmark` example no longer writes its test entries into your real history.
- The audio input device is released while listening is paused.
- HTTP 429 from the recognition service is reported as rate limiting instead of "no match".
- A poisoned ring-buffer lock no longer panics the capture path.

### Migrating from shazam-daemon

1. Stop and disable the old service: `systemctl --user disable --now shazam-daemon`.
2. Install hark. If you run the daemon through systemd, install `contrib/systemd/hark.service` and run `systemctl --user enable --now hark`.
3. Update everything that used the old names:
   - `playerctl -p Shazam` (or `-i Shazam`) becomes `playerctl -p hark` (or `-i hark`).
   - Metadata keys `shazam:*` become `hark:*`.
   - Scripts that read `$XDG_RUNTIME_DIR/shazam-daemon/...` should read `$XDG_RUNTIME_DIR/hark/...` with the new file names listed above, or use `contrib/waybar/hark-waybar`.
   - Replace the old Waybar module with the one in `contrib/waybar/` (module `custom/hark`, CSS id `#custom-hark`).
   - Any list that ignores or filters the player name "Shazam" needs to use "hark".
   - Anything that used `--download-current`, `--stream`, `--play` or the download D-Bus methods needs another tool, because hark no longer includes them.
4. History is carried over automatically on the first start. Once everything works you can delete the old `shazam_history.*` files.
