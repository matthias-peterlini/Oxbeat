# Oxbeat M2–M7 Roadmap Plan

**Goal:** Close every "Coming soon" in the app and reach a GarageBand-class feature set:
sounds and libraries, drums, voice tracks, effects and mixer, editing, export.

**Spec:** `docs/superpowers/specs/2026-11-09-oxbeat-design.md`

## Global constraints

- Audio thread never allocates, locks, frees or does I/O. Heavy objects (instruments,
  sample data, effect chains, audio clips) are built on the UI/loader side, sent in a
  command, and the replaced object comes back in an event to be dropped off the audio thread.
- Every new DSP unit has unit tests (silence in → silence out, bounded output, release ends).
- Project schema stays backward compatible: new fields are `#[serde(default)]`; old
  `preset: "Keys"` keeps meaning the built-in Keys synth.
- Only redistributable sounds; each pack carries its license.

## Architecture decisions

- **Instrument** (`engine/instrument.rs`): enum `Synth | Drums | Sampler | SoundFont`,
  renders stereo (adds into L/R). Sent with `EngineCommand::SetInstrument`.
- **Sound id** (string stored in `TrackKind::Instrument { preset }` / `Drums { kit }`):
  `Keys` (built-in), `builtin:<name>`, `sf2:<path>#<bank>:<program>`, `sfz:<path>`,
  `audio:<path>`, `pack:<pack-id>/<relative file>[#bank:program]`.
- **Sound library** (`sounds/`): built-in list, installed packs in
  `dirs::data_dir()/Oxbeat/packs/<id>/pack.json`, SFZ parser, SF2 via rustysynth,
  decoding via symphonia, loader cache of `Arc<SampleData>`.
- **Pack index:** `https://github.com/matthias-peterlini/Oxbeat-Sounds/releases/download/packs/index.json`;
  packs are zip release assets with sha256 in the index. Built by a workflow in the
  sounds repo from upstream sources (`sounds-repo/` here until that repo exists).
- **Voice tracks:** cpal input stream → rtrb ring → engine. Engine monitors through the
  track's effects and pushes recorded frames to a writer thread (hound WAV in the
  project bundle `audio/`). Audio clips reference a file; decoded data sent to the engine.
- **Effects:** per-track chain in the project (`Vec<EffectSlot{kind, params, on}>`), DSP
  state built on the UI side (`SetEffects`). Reverb, delay, EQ, compressor, distortion,
  chorus, noise gate, de-esser, pitch correction.
- **Editing:** undo/redo = snapshot stack of `Project`; clip move/trim/duplicate/delete;
  piano roll; beat grid for drum clips; quantize.
- **Transport extras:** metronome, count-in, cycle loop.
- **Export:** offline render through `Engine` → WAV (hound) and MP3.

## Milestones (one commit each, tests green, clippy clean)

1. Stereo instrument abstraction + synthesized drum kit + Drums track + drum pads strip.
2. Sampler, SFZ, SF2, audio import, sound browser dialog, pack manager/downloader,
   sounds-repo scaffolding (index builder workflow + README).
3. Voice tracks: input device, level meter, monitoring, recording takes, audio clip playback.
4. Effects (all listed) + effects panel + mixer panel + master.
5. Editing: undo/redo, clip ops, piano roll, beat grid, quantize.
6. Metronome, count-in, loop; export WAV/MP3; README; release.
