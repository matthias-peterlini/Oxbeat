# Oxbeat — Design

Date: 2026-11-09
Repo: https://github.com/matthias-peterlini/Oxbeat

## Intent

Oxbeat is a simple music-making app — "GarageBand for everyone." GarageBand has the right
feel but only runs on Mac; FL Studio can do everything but is complicated. Oxbeat runs on
**Windows, macOS and Linux** with the GarageBand "open it and make music" feel, while still
offering a solid set of functions.

**Success:** someone who has never used a music app plugs in a MIDI keyboard and is making a
song within five minutes. Typical first session: open app → pick some sounds → add an
instrument track → plug in keyboard, keys simply work → later add a vocal track from the mic
and put effects on it.

## Goals

- Cross-platform desktop app (Windows, macOS, Linux) from a single Rust codebase.
- Instant playing: a connected MIDI keyboard works with no setup; computer keyboard works as
  a fallback.
- Built-in instruments out of the box, plus compatibility with common free sound-library
  formats, plus an in-app downloader for curated sound packs hosted on GitHub.
- GarageBand-level feature set (below), presented without clutter.
- Automatic release builds for all three platforms via GitHub Actions.

## Non-goals (for now)

- Third-party plugins (VST/AU/CLAP). A native Oxbeat plugin system may come later.
- Commercial library formats (Kontakt, etc.) and any sounds we can't legally redistribute.
- Video, notation/score view, collaboration, cloud sync, mobile.

## Anti-goals

- Becoming FL Studio: every feature must fit without cluttering the main screen. If a feature
  can't be made simple, it waits.
- Any setup step between "plug in keyboard" and "hear sound".

## Feature set (agreed)

- **Tracks timeline:** rows of tracks with clips you can drag, copy, loop, trim.
- **Instrument tracks:** play via MIDI keyboard (auto-detected, hot-plug) or computer keyboard.
- **Audio (mic) tracks:** record the mic, input monitoring ("hear myself"), multiple takes.
- **Piano roll:** edit recorded notes; quantize.
- **Beat grid:** simple step sequencer for drums.
- **Basics:** tempo, key, metronome, count-in, loop (cycle) recording.
- **Mixer:** volume, pan, mute, solo per track (plus master).
- **Effects:** reverb, delay, EQ, compressor, distortion, chorus; vocal set: noise gate,
  de-esser, pitch correction.
- **Sound browser:** built-in instruments + downloadable GitHub sound packs.
- **Projects:** save/open, undo/redo, export to **WAV and MP3**.

## Sounds and libraries (agreed)

- Ships with a starter set: piano, drums, bass, synths — playable immediately.
- Supported import formats: **SoundFont (.sf2)**, **SFZ**, and plain audio files
  (WAV, MP3, FLAC, OGG) as one-shots/loops (mainly drums).
- A separate curated GitHub repo of **sound packs** (free, redistributable instruments,
  repackaged to load cleanly). Lots of them over time.
- In-app **sound browser** reads that repo's index: list packs → click download → pack appears
  in the instrument list. No leaving the app, no manual unzipping. *(Builder's call / my
  guess: checks for new packs on startup, non-blocking, works offline with what's installed.)*
- Only content whose license permits redistribution goes into packs; each pack carries its
  license/attribution.

## Main screen — "Focus" layout (agreed, chosen from mockups)

- **Top bar:** rewind / play / record, an LCD-style display (position · tempo · key; click
  tempo/key to change), and **+ Track**.
- **Middle:** roomy track rows. Header shows instrument icon + name; mute, solo, volume
  appear on hover. Clips are color-coded by track type; drag/trim/copy/loop.
- **Bottom strip:** context for the selected track — a live on-screen keyboard that lights up
  as you play (instrument track), or an input level meter + "hear myself" toggle (mic track).
- **Slide-over panels**, only when asked for:
  - **Sounds** — built-in + downloaded instruments, "Get more" (GitHub packs).
  - **Editor** — double-click a clip → piano roll (or beat grid for drum tracks).
  - **Effects** — the selected track's effect chain; **+ Add effect**, simple knobs.
  - **Mixer** — all tracks side by side.
- **+ Track** asks one thing: *Instrument*, *Mic*, or *Drums* — then you're playing.
- MIDI input auto-routes to the selected track *(my call)*.
- Dark theme *(my call)*; light theme later.

## How it gets made

Decided by the user:
- Language: **Rust**.
- Name: **Oxbeat**.
- Hosting: GitHub repo above; releases built and published there.

My calls (builder may revise with reason):
- **GUI:** `egui`/`eframe` — immediate-mode, cross-platform, well suited to custom-drawn
  timelines, piano rolls and knobs.
- **Audio I/O:** `cpal` (WASAPI / CoreAudio / ALSA+PulseAudio/PipeWire). Real-time audio
  thread must not allocate or lock; UI ↔ engine via lock-free queues (e.g. `rtrb`) or
  similar.
- **MIDI:** `midir`, with periodic re-scan for hot-plug.
- **Decoding:** `symphonia` (WAV/MP3/FLAC/OGG). WAV write: `hound`.
- **SF2:** `rustysynth` (or equivalent). **SFZ:** own loader for the common opcode subset
  (regions, key/vel ranges, loop, envelope basics).
- **MP3 export:** LAME-based encoder (LGPL) statically linked — fine because the app is
  GPL (see license below).
- **Effects and pitch correction:** implemented in-house (pitch correction: pitch detection
  e.g. YIN + correction toward the project key's scale).
- **Project format:** a folder/bundle containing a JSON (serde) project file plus recorded
  audio; versioned schema.
- **Sound-pack repo:** separate repo (`matthias-peterlini/Oxbeat-Sounds`) with an
  `index.json` listing packs (name, category, size, license, download URL, checksum); pack
  archives attached as GitHub Release assets (avoids repo file-size limits). App fetches over
  HTTPS, verifies checksum, extracts to the user data dir.
- **Built-in instruments:** small enough to ship inside the installer; larger packs are
  downloads.
- **Releases:** GitHub Actions on tag push → Windows installer (.msi or NSIS .exe), macOS
  .dmg (universal or x64+arm64), Linux AppImage + .deb. Code signing / notarization is
  out of scope until certificates exist (unsigned builds will show OS warnings).
- **License:** the user decided Oxbeat is **free and open source**. My call on which
  license: **GPL-3.0-or-later**, which keeps forks open and allows LGPL/GPL dependencies
  (e.g. LAME) to be linked statically. Dependencies must be GPL-3.0-compatible.
- **Sound-pack repo:** created by the user at
  https://github.com/matthias-peterlini/Oxbeat-Sounds

## Settled after builder check (my calls)

- **Latency:** target ≤ ~10 ms output buffer where the device allows. Windows default is
  WASAPI (shared); a "low-latency mode" toggle may use exclusive mode. ASIO is out of v1.
  macOS CoreAudio and Linux PipeWire/JACK/ALSA via cpal.
- **Live effects:** the full effect chain, including the vocal set, runs live in the
  input-monitoring path. Pitch correction is a live, low-latency effect (small lookahead
  accepted, latency-compensated on playback) — not an offline process.
- **Takes:** v1 keeps every take of a cycle recording and lets you pick one per clip.
  Comping (splicing parts of takes) is later, but the clip data model stores takes as a list
  under a clip so comping can be added without a schema break.
- **Plan scope:** the plan covers the full feature list, split into milestones; milestone 1
  is the core loop below. Engine graph is designed up front for per-track effect chains plus
  a master bus (sends/buses later).

## Left to the builder

- Internal architecture (engine graph, sequencing, timing), crate layout, test strategy.
- Exact built-in sound sources (must be redistributable).
- Visual polish, icons, keyboard shortcuts.
- Order of delivery — but the first usable milestone should be the core loop: open app →
  add instrument track → MIDI keyboard plays a built-in sound → record → play back → save.
