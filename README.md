# Oxbeat

Simple, friendly music making for **Windows, macOS and Linux** — the "open it and make music"
feel of GarageBand, everywhere. Plug in a MIDI keyboard and play; record, edit, add your voice,
and export your song.

> Status: early development (Milestone 1 — core loop).

## Download

Grab the installer for your system from the
[Releases page](https://github.com/matthias-peterlini/Oxbeat/releases).

## Build from source

Requires [Rust](https://rustup.rs) (stable).

Linux also needs: `libasound2-dev libudev-dev libxkbcommon-dev libgtk-3-dev`.

```bash
cargo run --release
```

## Sounds

Extra instruments and sound packs live in
[Oxbeat-Sounds](https://github.com/matthias-peterlini/Oxbeat-Sounds) and can be downloaded from
inside the app.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
