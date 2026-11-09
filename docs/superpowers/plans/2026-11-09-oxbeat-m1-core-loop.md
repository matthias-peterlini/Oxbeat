# Oxbeat Milestone 1 — Core Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open Oxbeat → add an instrument track → a MIDI keyboard (or computer keyboard) plays a built-in sound → record → play back → save/open — on Windows, macOS and Linux, with tagged GitHub releases built automatically.

**Architecture:** One Rust binary crate. A pure, allocation-free `Engine` renders audio blocks from a `Project` snapshot plus live events; it's driven by a `cpal` output stream and receives commands from the UI/MIDI threads over a lock-free `rtrb` queue, sending recorded notes and status back on a second queue. The `eframe`/`egui` UI owns the `Project` (source of truth), and pushes a fresh `Arc<Project>` snapshot to the engine on every edit.

**Tech Stack:** Rust (stable, edition 2021), eframe/egui, cpal, midir, rtrb, serde + serde_json, rfd (file dialogs), anyhow/thiserror, cargo-packager (installers), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-09-oxbeat-design.md`

## Global Constraints

- App name **Oxbeat**; license **GPL-3.0-or-later**; every dependency GPL-3.0-compatible.
- Must build and run on Windows, macOS, Linux from one codebase.
- No setup step between "plug in keyboard" and "hear sound": MIDI auto-detected, hot-plug, routed to the selected track.
- Audio callback never allocates, locks, or does I/O.
- Target output buffer ≤ ~10 ms where the device allows (request 256 frames, fall back to device default).
- UI: "Focus" layout — top bar (⏮ ▶ ● · LCD position/tempo/key · + Track), roomy track rows, bottom keyboard strip; dark theme.
- + Track asks one question: Instrument / Mic / Drums. In M1 only Instrument is enabled; Mic and Drums are shown disabled with tooltip "Coming soon".
- Project = folder `<Name>.oxbeat/` containing `project.json` (schema `version: 1`).
- Use crate versions current at implementation time; pin them in `Cargo.toml`.

## Review Focus

1. **No audio output device** (or device fails to open) → app still opens, shows a non-blocking banner "No audio device found", retries when the user clicks it. Test in Task 6.
2. **MIDI keyboard unplugged while keys are held** → no stuck notes; engine receives `AllNotesOff` for that track on disconnect. Test in Task 5 (engine) + Task 7 (rescan diff).
3. **Device runs at 44.1 kHz, not 48 kHz** → recorded/played note timing is still correct in beats. Test in Task 3 and Task 5.
4. **Opening a project with a newer or broken `project.json`** → clear error dialog, current project untouched, no crash. Test in Task 2.
5. **Variable callback block sizes** (cpal may hand any size, incl. 1 and 4096) → identical output to fixed-size rendering. Test in Task 5.

---

## File Structure

```
Cargo.toml
LICENSE                       GPL-3.0 text
README.md
assets/icon.png               app icon (512x512)
src/main.rs                   eframe entry point, wires everything
src/project/mod.rs            Project/Track/Clip/Note model
src/project/io.rs             save/load bundle
src/engine/mod.rs             Engine: render, command handling, record
src/engine/transport.rs       beat <-> sample math
src/engine/synth.rs           built-in polyphonic synth + presets
src/engine/commands.rs        EngineCommand / EngineEvent enums
src/audio/output.rs           cpal stream owning the Engine
src/midi/input.rs             midir ports, hot-plug, message parsing
src/midi/keyboard.rs          computer-keyboard -> MIDI pitch
src/ui/mod.rs                 App state, theme, layout root
src/ui/top_bar.rs
src/ui/tracks.rs
src/ui/keyboard_strip.rs
.github/workflows/ci.yml      test on 3 OSes
.github/workflows/release.yml tag -> installers -> GitHub Release
```

---

### Task 1: Scaffold, window, CI

**Files:** Create `Cargo.toml`, `LICENSE`, `README.md`, `.gitignore` (exists), `src/main.rs`, `src/ui/mod.rs`, `.github/workflows/ci.yml`

**Interfaces:**
- Produces: `ui::OxbeatApp` implementing `eframe::App`; `ui::apply_theme(ctx: &egui::Context)`.

- [ ] **Step 1:** `cargo init --name oxbeat`; set `license = "GPL-3.0-or-later"`, `edition = "2021"`, `repository = "https://github.com/matthias-peterlini/Oxbeat"`. Add LICENSE (GPL-3.0 full text), README (one paragraph + build instructions).
- [ ] **Step 2: Write failing test** in `src/ui/mod.rs`:
```rust
#[test]
fn theme_is_dark() {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    assert!(ctx.style().visuals.dark_mode);
}
```
- [ ] **Step 3:** Run `cargo test theme_is_dark` → FAIL (not defined).
- [ ] **Step 4:** Implement `apply_theme` (dark visuals, background `#16181d`, panel `#22252c`, accent `#4b8be0`, record red `#d9434f`) and an empty `OxbeatApp`; `main.rs` opens a 1200×760 window titled "Oxbeat".
- [ ] **Step 5:** `cargo test` → PASS; `cargo run` shows an empty dark window.
- [ ] **Step 6:** `ci.yml`: on push/PR, matrix `ubuntu-latest, windows-latest, macos-latest`; Ubuntu installs `libasound2-dev libudev-dev libxkbcommon-dev libgtk-3-dev`; runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- [ ] **Step 7:** Commit `feat: scaffold Oxbeat window and CI`.

### Task 2: Project model and save/load

**Files:** Create `src/project/mod.rs`, `src/project/io.rs`

**Interfaces:**
- Produces:
  - `struct Project { version: u32, name: String, tempo_bpm: f64, key: String, tracks: Vec<Track> }` (`Default`: version 1, "Untitled", 120.0, "C major", empty)
  - `struct Track { id: u64, name: String, kind: TrackKind, volume: f32 /*0..=1.5, default 0.8*/, pan: f32 /*-1..=1*/, mute: bool, solo: bool, clips: Vec<Clip> }`
  - `enum TrackKind { Instrument { preset: String }, Audio, Drums }`
  - `struct Clip { id: u64, start_beat: f64, length_beats: f64, notes: Vec<Note> }`
  - `struct Note { pitch: u8, velocity: u8, start_beat: f64 /*relative to clip*/, length_beats: f64 }`
  - `Project::add_instrument_track(&mut self, preset: &str) -> u64` (name = preset, unique id)
  - `io::save(project: &Project, bundle_dir: &Path) -> anyhow::Result<()>` writes `bundle_dir/project.json` (pretty JSON), creating the dir.
  - `io::load(bundle_dir: &Path) -> Result<Project, LoadError>`; `enum LoadError { Io(..), Parse(..), TooNew { found: u32 } }`.
  - All structs derive `Serialize, Deserialize, Clone, Debug, PartialEq`.

- [ ] **Step 1: Write failing tests** (use `tempfile`):
```rust
#[test] fn roundtrip() { /* project with 1 track, 1 clip, 2 notes; save then load; assert_eq! */ }
#[test] fn rejects_newer_version() { /* write {"version":2,...}; load -> Err(LoadError::TooNew{found:2}) */ }
#[test] fn broken_json_is_parse_error() { /* write "{oops"; load -> Err(LoadError::Parse(_)) */ }
#[test] fn track_ids_unique() { let mut p = Project::default(); let a = p.add_instrument_track("Keys"); let b = p.add_instrument_track("Keys"); assert_ne!(a, b); }
```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat: project model and bundle save/load`.

### Task 3: Transport math

**Files:** Create `src/engine/transport.rs`

**Interfaces:**
- Produces: `fn samples_per_beat(tempo_bpm: f64, sample_rate: u32) -> f64`; `fn beat_to_sample(beat: f64, tempo_bpm: f64, sample_rate: u32) -> u64` (floor); `fn sample_to_beat(sample: u64, tempo_bpm: f64, sample_rate: u32) -> f64`.

- [ ] **Step 1: Failing tests:**
```rust
assert_eq!(samples_per_beat(120.0, 48_000), 24_000.0);
assert_eq!(beat_to_sample(4.0, 120.0, 44_100), 88_200);
assert!((sample_to_beat(88_200, 120.0, 44_100) - 4.0).abs() < 1e-9);
assert_eq!(beat_to_sample(1.0, 90.0, 48_000), 32_000);
```
- [ ] **Step 2–4:** fail → implement → pass. **Step 5:** Commit `feat: transport beat/sample math`.

### Task 4: Built-in synth

**Files:** Create `src/engine/synth.rs`

**Interfaces:**
- Produces:
  - `const PRESETS: [&str; 4] = ["Keys", "Bass", "Lead", "Pad"];`
  - `struct Synth` — 16-voice polyphonic, preallocated; `Synth::new(preset: &str, sample_rate: u32) -> Synth` (unknown preset → "Keys"); `note_on(&mut self, pitch: u8, velocity: u8)`, `note_off(&mut self, pitch: u8)`, `all_notes_off(&mut self)`, `render(&mut self, out: &mut [f32])` (mono, **adds** into `out`), `active_voices(&self) -> usize`.
  - Voice: oscillator(s) + ADSR + one-pole low-pass. Presets: Keys = triangle+sine, A 5 ms D 300 ms S 0.4 R 250 ms; Bass = saw, LP cutoff 800 Hz, A 2 ms R 80 ms; Lead = square, A 5 ms S 0.8 R 120 ms; Pad = 2 detuned saws (±7 cents), A 400 ms R 800 ms. Voice stealing: oldest voice. Velocity scales amplitude linearly. Peak per voice ≤ 0.25.

- [ ] **Step 1: Failing tests:**
```rust
#[test] fn silent_without_notes() { let mut s = Synth::new("Keys", 48_000); let mut b = [0.0f32; 512]; s.render(&mut b); assert!(b.iter().all(|x| *x == 0.0)); }
#[test] fn note_makes_sound() { /* note_on(60,100); render 512; max abs > 0.01 */ }
#[test] fn release_goes_silent() { /* note_on, render 4800, note_off, render 1s; active_voices()==0, last block all zero */ }
#[test] fn steals_beyond_16() { /* 20 note_ons -> active_voices()==16 */ }
#[test] fn all_notes_off_releases() { /* hold 3 notes, all_notes_off, render 1s -> active_voices()==0 */ }
```
- [ ] **Step 2–4:** fail → implement → pass. **Step 5:** Commit `feat: built-in polyphonic synth presets`.

### Task 5: Engine (render, play, record)

**Files:** Create `src/engine/mod.rs`, `src/engine/commands.rs`

**Interfaces:**
- Consumes: Task 2 model, Task 3 math, Task 4 `Synth`.
- Produces:
  - `enum EngineCommand { SetProject(Arc<Project>), NoteOn { track: u64, pitch: u8, velocity: u8 }, NoteOff { track: u64, pitch: u8 }, AllNotesOff { track: u64 }, Play, Stop, Record { track: u64 }, Locate(f64 /*beat*/) }`
  - `enum EngineEvent { Position(f64), Recorded { track: u64, clip: Clip } }`
  - `struct Engine` ; `Engine::new(sample_rate: u32, cmd_rx: rtrb::Consumer<EngineCommand>, evt_tx: rtrb::Producer<EngineEvent>) -> Engine`; `fn process(&mut self, out: &mut [f32], channels: usize)` (interleaved).
- Behavior: one `Synth` per instrument track, created when a `SetProject` introduces a track (the drop of old `Arc`s/synths must not happen on the audio thread — send replaced values back via a garbage queue or `basedrop`-style deferral). Playback schedules clip notes sample-accurately within the block. Mixing: equal-power pan, track volume, mute; if any track is solo, only solo tracks sound. Output clamped to [-1, 1]. Recording: live NoteOn/NoteOff on the record track are timestamped in beats from the transport; on `Stop` (or `Play` toggle) emits `Recorded` with one clip starting at the record start beat, length rounded up to a whole bar (4 beats), notes still held get closed at stop. Emits `Position` at most every ~1/30 s. Live notes always sound, even when stopped.

- [ ] **Step 1: Failing tests** (drive with a real `rtrb` pair; helper `render(engine, frames) -> Vec<f32>`):
```rust
#[test] fn live_note_sounds_when_stopped()
#[test] fn muted_track_is_silent()
#[test] fn solo_silences_others()
#[test] fn clip_note_starts_on_exact_sample()   // note at beat 1, 120 bpm, 48k: frames < 24_000 silent, sample 24_000.. non-zero
#[test] fn timing_correct_at_44100()           // same at 44.1k -> first non-zero frame == 22_050
#[test] fn record_captures_note_in_beats()     // Record, NoteOn at frame 24_000, NoteOff at 48_000, Stop -> Recorded clip, note start 1.0 len 1.0, clip len 4.0
#[test] fn held_note_closed_on_stop()
#[test] fn all_notes_off_stops_live_notes()
#[test] fn block_size_independent()            // rendering 2s in blocks of 1, 37, 4096 == one 96_000-frame block (bitwise)
#[test] fn output_is_clamped()                 // 16 loud notes on 4 tracks -> all |x| <= 1.0
```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat: audio engine with playback and recording`.

### Task 6: Audio output

**Files:** Create `src/audio/output.rs`

**Interfaces:**
- Consumes: `Engine`.
- Produces: `struct AudioOutput` (holds the `cpal::Stream`); `fn start(cmd_rx, evt_tx) -> Result<AudioOutput, AudioError>`; `enum AudioError { NoDevice, Build(String) }`; `AudioOutput::sample_rate(&self) -> u32`.
- Picks default output device, f32 if available (convert otherwise), requests `BufferSize::Fixed(256)` and retries with `Default` if rejected.

- [ ] **Step 1: Failing test:** `fn error_message_for_no_device()` — `AudioError::NoDevice.to_string() == "No audio device found"`.
- [ ] **Step 2–4:** fail → implement → pass.
- [ ] **Step 5: Manual check:** temporary `main` path sends `NoteOn` to a "Keys" track for 1 s → audible tone on the dev machine.
- [ ] **Step 6:** Commit `feat: cpal audio output`.

### Task 7: MIDI and computer keyboard input

**Files:** Create `src/midi/input.rs`, `src/midi/keyboard.rs`

**Interfaces:**
- Produces:
  - `enum MidiMsg { NoteOn { pitch: u8, velocity: u8 }, NoteOff { pitch: u8 } }`
  - `fn parse(bytes: &[u8]) -> Option<MidiMsg>` (any channel; NoteOn vel 0 → NoteOff; others → None)
  - `struct MidiInputs` — `MidiInputs::new(sink: Arc<dyn Fn(MidiMsg) + Send + Sync>) -> Self`; `fn rescan(&mut self) -> RescanResult` connecting new ports, dropping vanished ones; `struct RescanResult { added: Vec<String>, removed: Vec<String> }`. Caller rescans every 1 s and sends `AllNotesOff` for the selected track when `removed` is non-empty.
  - `fn diff_ports(old: &[String], new: &[String]) -> RescanResult` (pure, tested).
  - `fn key_to_pitch(key: egui::Key, octave: i8) -> Option<u8>` — row `A W S E D F T G Y H U J K` = semitones 0..=12 from C of `octave` (octave 4 → A = 60); `Z`/`X` change octave (handled by caller).

- [ ] **Step 1: Failing tests:**
```rust
assert_eq!(parse(&[0x90, 60, 100]), Some(MidiMsg::NoteOn { pitch: 60, velocity: 100 }));
assert_eq!(parse(&[0x93, 60, 0]), Some(MidiMsg::NoteOff { pitch: 60 }));
assert_eq!(parse(&[0x80, 64, 10]), Some(MidiMsg::NoteOff { pitch: 64 }));
assert_eq!(parse(&[0xB0, 7, 100]), None);
assert_eq!(parse(&[0x90]), None);
assert_eq!(key_to_pitch(egui::Key::A, 4), Some(60));
assert_eq!(key_to_pitch(egui::Key::K, 4), Some(72));
assert_eq!(key_to_pitch(egui::Key::Q, 4), None);
let r = diff_ports(&["A".into(), "B".into()], &["B".into(), "C".into()]);
assert_eq!((r.added, r.removed), (vec!["C".to_string()], vec!["A".to_string()]));
```
- [ ] **Step 2–4:** fail → implement → pass.
- [ ] **Step 5:** Commit `feat: MIDI hot-plug input and computer keyboard`.

### Task 8: Focus UI wired end-to-end

**Files:** Modify `src/main.rs`, `src/ui/mod.rs`; Create `src/ui/top_bar.rs`, `src/ui/tracks.rs`, `src/ui/keyboard_strip.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: `OxbeatApp::new(cc) -> Self` owning `Project`, selected track id, `rtrb` producer/consumer, `Option<AudioOutput>`, `MidiInputs`, held-notes set (`[bool; 128]`) for the strip, octave.
- Pure helpers (tested): `fn format_position(beat: f64) -> String` (`0.0 → "001.1"`, `5.0 → "002.2"`; bar.beat, 4/4); `OxbeatApp::apply_event(&mut self, e: EngineEvent)` appends `Recorded` clips to the matching track and re-sends `SetProject`.

Layout: top bar (⏮ Locate(0) · ▶/■ toggle · ● record on selected track · LCD `format_position · {tempo} BPM · {key}`, tempo editable by drag · **+ Track** popup with three big buttons Instrument / Mic / Drums — Mic/Drums disabled "Coming soon"; Instrument then lists the 4 presets). Track rows 56 px tall: icon + name, hover reveals M/S buttons and volume slider; clips drawn as green rounded rects with mini note lines; clicking a row selects it. Bottom strip: 2-octave on-screen keyboard; keys light when held (MIDI or computer keyboard); mouse-clicking a key plays it. Menu shortcut `Ctrl/Cmd+S` save (rfd folder picker on first save), `Ctrl/Cmd+O` open; load errors show an egui modal with the `LoadError` message. Banner "No audio device found — click to retry" when `AudioOutput::start` failed. First launch: project starts with one "Keys" instrument track selected, so playing works immediately.

- [ ] **Step 1: Failing tests:** `format_position` cases above; `apply_event_adds_recorded_clip` (track gains 1 clip).
- [ ] **Step 2–4:** fail → implement → pass.
- [ ] **Step 5: Manual end-to-end check** (`cargo run --release`): launch → press `A S D` → hear Keys; plug a MIDI keyboard → plays without any clicks; unplug while holding a key → sound stops; ● record, play 4 notes, ■ → clip appears; ⏮ ▶ → notes replay in time; Ctrl+S, quit, reopen, Ctrl+O → clip still there.
- [ ] **Step 6:** Commit `feat: Focus layout UI wired to engine and MIDI`.

### Task 9: Release pipeline

**Files:** Modify `Cargo.toml` (`[package.metadata.packager]`), Create `.github/workflows/release.yml`, `assets/icon.png`

- [ ] **Step 1:** Configure cargo-packager: product name "Oxbeat", identifier `io.github.matthias-peterlini.oxbeat`, icon, formats: `nsis` (Windows), `dmg` (macOS), `appimage` + `deb` (Linux).
- [ ] **Step 2:** `release.yml`: on push of tags `v*`; matrix windows-latest / macos-latest (build `aarch64-apple-darwin` and `x86_64-apple-darwin`, one dmg each) / ubuntu-22.04 (for older glibc); `cargo build --release`, `cargo packager --release`; upload artifacts to a GitHub Release for the tag (`softprops/action-gh-release`), release notes auto-generated. Unsigned (spec: signing out of scope).
- [ ] **Step 3: Verify locally:** `cargo packager --release --formats nsis` produces an installer that installs and launches Oxbeat on Windows.
- [ ] **Step 4:** Commit `ci: release installers for Windows, macOS, Linux`; push `main`; push tag `v0.1.0`; confirm the Release page lists `.exe`, two `.dmg`, `.AppImage`, `.deb`.

---

## Later milestones (each gets its own plan)

- **M2 Sounds:** SF2 + SFZ + audio-file instruments, built-in piano/drums/bass, Sounds panel, `Oxbeat-Sounds` repo + in-app downloader.
- **M3 Editing:** clip drag/trim/copy/loop, piano roll + quantize, undo/redo, metronome, count-in, cycle recording, tempo/key editing.
- **M4 Mic tracks:** audio recording, input monitoring, takes.
- **M5 Effects & mixer:** effect chains (reverb, delay, EQ, compressor, distortion, chorus, gate, de-esser, pitch correction), master bus, Mixer panel.
- **M6 Beat grid:** Drums tracks with step sequencer.
- **M7 Export:** WAV + MP3 (LAME).
