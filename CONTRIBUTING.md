# Contributing to hark

Thanks for your interest in hark. Bug reports, fixes and focused improvements are all welcome.

Before you start:

- For anything larger than a bug fix, please open an issue first so the approach can be agreed on before you spend time on it.
- Security problems are handled privately. See [SECURITY.md](SECURITY.md) instead of opening a public issue.

## Development setup

You need:

- Rust 1.88 or newer (stable).
- ALSA development headers and `pkg-config` to build:
  - Debian and Ubuntu: `sudo apt install libasound2-dev pkg-config`
  - Arch: `sudo pacman -S alsa-lib pkgconf`
  - Fedora: `sudo dnf install alsa-lib-devel pkgconf-pkg-config`
- To run the daemon: a sound server that exposes ALSA (PipeWire with `pipewire-alsa` or `pipewire-pulse`, or PulseAudio) and a D-Bus session bus.

```sh
git clone https://github.com/omsingh02/hark.git
cd hark
cargo build
```

## Checks

CI runs these four commands on every pull request. Please run them locally first:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
```

The unit tests need neither audio hardware nor network access.

## Running from source

```sh
cargo run --release -- --source mic
```

This starts the daemon in the foreground. It listens to the selected input and sends audio fingerprints to the recognition service while music is playing, so only run it when you are comfortable with that. Only one instance can run at a time: if you installed the systemd unit, stop it first with `systemctl --user stop hark`.

## Examples

The programs in `examples/` are developer tools. They are not installed by `cargo install`.

```sh
# Micro-benchmarks: resampling, silence detection, fingerprinting, ring buffer, history storage.
# The history benchmark uses a temporary directory, never your real history.
cargo run --release --example benchmark

# Records six seconds from the default input, prints level and fingerprint diagnostics and
# posts the fingerprint to the recognition service. Writes the raw capture to /tmp/captured_live.pcm.
cargo run --example test_capture
```

## Repository layout

```
src/                 the daemon
  audio/             capture, resampling, silence detection
  cache/             cover-art cache
  history/           recognition history (JSONL and plain text)
  mpris/             D-Bus MPRIS2 service
  network/           recognition-service client
crates/hark-core/    fingerprinting and response models (library crate)
examples/            benchmark and capture diagnostics
contrib/             systemd unit, Waybar and Hyprland examples
assets/              logo and banner
```

## Commit messages

The history uses [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): summary`, with the scope optional. The history mostly uses `feat`, `fix`, `refactor`, `docs`, `chore` and `style`; `test`, `ci`, `perf` and `build` are fine where they fit. Examples:

```
fix(audio): release the capture device while paused
feat(history): dedupe, compact and delete history entries
docs: describe the workspace layout
```

Write the summary in the imperative mood and use the body to explain why. Mark breaking changes with `!` after the type or a `BREAKING CHANGE:` footer.

## Pull requests

- Keep each pull request focused on one change.
- Add or update tests for behaviour changes and bug fixes.
- Update the documentation and add an entry under "Unreleased" in [CHANGELOG.md](CHANGELOG.md) for anything users will notice. Changes to command-line flags, files on disk or the D-Bus interface should be called out as such.
- Make sure the four checks above pass.

## Licensing

hark is licensed under GPL-3.0-or-later, and contributions are accepted under the same license. By submitting a pull request you confirm that you have the right to license your work this way. Please do not copy in code from projects whose licenses are incompatible with the GPL.

The fingerprinting code in `crates/hark-core/src/dsp` is derived from [SongRec](https://github.com/marin-m/SongRec) by marin-m, which is GPL-3.0-or-later. Changes there must stay under the GPL, and the attribution comments at the top of those files should be kept.
