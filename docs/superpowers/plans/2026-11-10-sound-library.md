# Sound Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Oxbeat real instruments out of the box. The "Oxbeat Essentials" pack downloads automatically on first launch, and after that a new Keys track plays a sampled grand piano.

**Architecture:** The app side lives in this repo (`Rec`, the Oxbeat app). It gets:
- compact in-memory sample storage;
- per-sound categories;
- an Essentials auto-installer built on the existing `PackManager`;
- default sounds that follow Essentials;
- a credits window.

The pack side lives in the sibling checkout `../Oxbeat-Sounds` (GitHub repository `Oxbeat-beats`). It gets:
- a build tool that handles multi-source recipes with a preparation script;
- the `essentials` recipe;
- new Store recipes.

**Tech Stack:** Rust 2024, egui/eframe, symphonia (decoding), rustysynth (SF2), Node 18+ (`tools/build.mjs`), `flac` CLI in CI.

**Spec:** `docs/superpowers/specs/2026-11-10-sound-library-design.md`

## Global Constraints

- **Licences.** Only CC0, public domain, CC BY, CC BY-SA, MIT and the GeneralUser GS licence are allowed. Never NC, Sampling Plus, or an unstated licence.
- **Essentials download size:** ≤ 250 MB.
- **Essentials RAM when fully loaded:** under about 600 MB.
- **Saved projects never change sound.** The serde default kit stays `"Studio Kit"`, and the id `"Keys"` stays the synth.
- **The app never blocks on the network.** The window opens at once, and the synths work offline forever.
- **Removal is respected.** Once the user removes Essentials, it is never downloaded again automatically.
- **Pack ids** match `^[a-z0-9_-]{1,64}$`. The Essentials id is `essentials`.
- **Categories** are exactly: `Keyboards`, `Strings`, `Guitars`, `Bass`, `Winds`, `Brass`, `Percussion`, `Drums`, `Synths`, `World`.
- **Test command:** `cargo test` in `Rec`, and `node tools/build.mjs --only <id> --no-previous` in `Oxbeat-Sounds`.
- **Commits** end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Nothing is pushed or tagged without the user's explicit OK.**

## Review Focus

1. **First launch offline.** No panic, no modal. The strip says there is no connection, the Keys synth plays, and the next launch tries again.
2. **App closed mid-download.** No half-installed pack: the `.unpack-*` temp dir and the partial zip are cleaned up. The next launch downloads again.
3. **User removes Essentials.** It is not re-downloaded. New tracks fall back to `"Keys"`. A project that uses `pack:essentials/...` reports a missing sound as today, without crashing.
4. **Compact samples at buffer edges.** Mono `i16` playback reads the last frame without going out of bounds, and loops wrap as they do with `f32`. Sources at 24-bit and 32-bit float are clamped when converted to `i16`.
5. **The starter track upgrade fires only on an untouched starter project.** One `"Keys"` track, no clips, not modified since launch. It must not fire on a loaded project or one the user has edited.

---

### Task 1: Compact sample storage

**Files:**
- Modify: `src/engine/sampler.rs:12-27` (SampleData), `src/engine/sampler.rs:271-334` (render)
- Modify: `src/engine/mod.rs:686-720` (mix_clip)
- Modify: `src/sounds/decode.rs` (add compact decode)
- Modify: `src/sounds/mod.rs:112-119` (`sample()` uses compact decode)
- Modify: every `SampleData { rate, frames: vec![..] }` literal: `src/sounds/sfz.rs:405`, `src/ui/studio.rs:1019`, `src/engine/tests.rs:431`, the sampler tests
- Modify: `src/ui/studio.rs:87-98` (Waveform::of), `src/ui/studio.rs:786-798` (write_wav)

**Interfaces:**
- Produces:
  ```rust
  pub enum Frames { Stereo(Vec<f32>), Compact { channels: u8, data: Vec<i16> } }
  pub struct SampleData { pub rate: u32, pub frames: Frames }
  impl SampleData {
      pub fn stereo(rate: u32, interleaved: Vec<f32>) -> SampleData;
      pub fn compact(rate: u32, channels: u8, data: Vec<i16>) -> SampleData; // channels 1 or 2
      pub fn len(&self) -> usize;          // frames
      pub fn is_empty(&self) -> bool;
      pub fn frame(&self, i: usize) -> (f32, f32); // mono returns (x, x)
      pub fn bytes(&self) -> usize;        // heap bytes of sample data
  }
  pub fn decode_compact(path: &Path) -> Result<SampleData, String>; // in sounds::decode
  ```
- `decode_file` keeps its signature and returns `Frames::Stereo`. Recordings and imported audio clips stay `f32`.

- [ ] **Step 1: Write failing tests**
  - In `sampler.rs` tests:
    - `compact_mono_frame_duplicates_channels`: `SampleData::compact(44_100, 1, vec![16384, -16384])` gives `len() == 2`, `frame(0) == (0.5, 0.5)`, `frame(1) == (-0.5, -0.5)` and `bytes() == 4`.
    - `compact_and_stereo_render_alike`: one sine as `Stereo` and as `Compact { channels: 1 }`, played through `Sampler` at the same key. The per-sample difference is ≤ `1.0 / 16384.0`.
  - In `decode.rs`:
    - `decode_compact_keeps_mono`: the existing mono WAV fixture gives `Frames::Compact { channels: 1, .. }` with `len() == 1000`.
    - `decode_compact_clamps`: a 32-bit float WAV holding `1.5` decodes to `i16::MAX`.
- [ ] **Step 2: Run `cargo test sampler decode`.** Expected: FAIL to compile, because `compact`, `frame` and `Frames` are not defined.
- [ ] **Step 3: Implement.**
  - Add `Frames` and the `SampleData` methods.
  - Switch `render` and `mix_clip` to read frames through the storage. Match on `frames` once per voice (or once per clip) per block, not once per sample. A small generic helper over a `frame` closure is fine.
  - Write `decode_compact`: same symphonia loop as `decode_file`, keep the native channel count (more than 2 channels are downmixed to stereo), and convert with `(x * 32767.0).round().clamp(-32768.0, 32767.0) as i16`.
  - Make `sounds::sample()` call `decode_compact`.
  - Rewrite `Waveform::of` and `write_wav` with `frame()`.
  - Replace the struct literals with `SampleData::stereo(..)`.
- [ ] **Step 4: Run `cargo test`.** Expected: all pass. That includes `every_builtin_loads`, the SFZ tests and the export tests.
- [ ] **Step 5: Commit** with the message `perf: keep instrument samples as 16-bit, mono stays mono`.

### Task 2: Categories and homepage in pack manifests

**Files:**
- Modify: `src/sounds/library.rs:37-66` (PackSound, PackManifest) and its tests
- Modify: `src/ui/sound_browser.rs:434-503` (Library tab)

**Interfaces:**
- Produces:
  - `PackSound.category: String` (`#[serde(default)]`);
  - `PackManifest.category: String` (`#[serde(default)]`);
  - `PackManifest.homepage: String` (`#[serde(default)]`);
  - `pub const CATEGORIES: [&str; 10]`, in the order of the Global Constraints;
  - `pub fn category_of<'a>(pack: &'a PackManifest, sound: &'a PackSound) -> &'a str`. It returns the sound's category, then the pack's category if that is one of `CATEGORIES`, then `"Synths"` for built-ins and `"Other"` for anything else.

- [ ] **Step 1: Write the failing test** `category_falls_back_to_pack`. Three manifests:
  - one whose sounds have no category and whose pack category is `"Woodwinds"` (not in the list): the result is `"Other"`;
  - one whose pack category is `"Strings"`: the result is `"Strings"`;
  - one whose sound category is `"Keyboards"`: the result is `"Keyboards"`.

  Also: old `pack.json` files without these fields still parse.
- [ ] **Step 2: Run `cargo test library`.** Expected: FAIL, because `category_of` is not defined.
- [ ] **Step 3: Implement the fields and `category_of`.** Then change the Library tab:
  - group every installed sound under `section_label(category)`, in `CATEGORIES` order, with "Other" last;
  - the row caption becomes `"{caption} · {pack.name}"`, or just the pack name when the caption is empty;
  - imports stay in their "Imported" section.
- [ ] **Step 4: Run `cargo test`.** Expected: PASS. Then run `cargo run`, open Add Track → Instrument → My Library, and check that GeneralUser sounds appear under "Other". GeneralUser has no categories yet; Task 6 gives them categories.
- [ ] **Step 5: Commit** with the message `feat: group library sounds by instrument`.

### Task 3: Essentials auto-install

**Files:**
- Create: `src/sounds/essentials.rs` (state file and decision logic, pure and tested)
- Modify: `src/sounds/mod.rs` (add `pub mod essentials;`)
- Modify: `src/sounds/packs.rs` (`PackManager`: drive the auto-install; mark removal in `uninstall`)
- Modify: `src/ui/mod.rs` (call per frame; draw the strip)
- Modify: `src/ui/sound_browser.rs` (an "Install instruments" row in the Library tab while Essentials is missing)

**Interfaces:**
- Produces in `sounds::essentials`:
  ```rust
  pub const ID: &str = "essentials";
  #[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
  pub struct State { pub removed_by_user: bool }
  pub fn load(dir: &Path) -> State;              // dir/essentials.json, default when missing or corrupt
  pub fn save(dir: &Path, s: &State);
  pub fn should_auto_install(installed: bool, downloading: bool, state: &State) -> bool;
  ```
- Produces on `PackManager`:
  ```rust
  pub fn ensure_essentials(&mut self);           // call every frame; cheap when nothing to do
  pub fn essentials_status(&self) -> EssentialsStatus;
  pub enum EssentialsStatus { Hidden, Downloading { permille: u32, size: u64 }, Failed(String) }
  ```
  `Failed` keeps the message from `packs::friendly`, for example "No internet connection.".
- `uninstall(ID)` saves `State { removed_by_user: true }`. A manual Download from the Store, or the "Install instruments" row, saves it back to `false`.

- [ ] **Step 1: Write failing tests** in `essentials.rs`:
  - `auto_install_only_when_missing_and_not_removed`: check the truth table of `should_auto_install`;
  - `state_round_trips_and_survives_garbage`: with a temp dir, `save` then `load` returns the same state, and garbage JSON gives `State::default()`.
- [ ] **Step 2: Run `cargo test essentials`.** Expected: FAIL, because the module is missing.
- [ ] **Step 3: Implement.** `ensure_essentials`:
  - when `should_auto_install` is true and the index is `NotLoaded`, call `load_index()`;
  - when the index is `Ready` and holds `ID`, call `install(entry)` once;
  - when the index is `Failed`, report `Failed`. Do not retry until the next launch; the Store's "Try again" still works;
  - when the index is `Ready` without `ID` (not published yet), stay `Hidden`.

  In `ui/mod.rs`, draw a slim, dismissible top strip:
  - while downloading: "Downloading the instrument library… 42% (95 of 230 MB)", with the size from the index entry;
  - on failure: "Couldn't download the instrument library: No internet connection. Oxbeat will try again next time.";
  - dismissing it only hides it for this session.

  When an install completes, `poll()` already refreshes `installed`, so the sounds show up without a restart.
- [ ] **Step 4: Run `cargo test`.** Expected: PASS. Then check by hand with `cargo run`:
  - delete `%APPDATA%\Oxbeat\packs\essentials` and start the app: the strip appears. Until Task 6 is published it stays Hidden;
  - disconnect from the network: the failure text appears and the app still plays.
- [ ] **Step 5: Commit** with the message `feat: download the instrument library on first launch`.

### Task 4: Default sounds follow Essentials

**Files:**
- Modify: `src/sounds/mod.rs` (default sound ids)
- Modify: `src/ui/studio.rs:220-225` (`starter_project`) and the place where `PackManager::poll` reports a finished install (`src/ui/mod.rs`)

**Interfaces:**
- Consumes: `essentials::ID`, `library::installed_packs`.
- Produces in `sounds`:
  ```rust
  pub const ESSENTIALS_PIANO: &str = "pack:essentials/Grand Piano.sfz";
  pub const ESSENTIALS_KIT: &str = "pack:essentials/Drums.sfz";
  pub fn default_instrument(essentials_installed: bool) -> &'static str; // ESSENTIALS_PIANO or "Keys"
  pub fn default_kit(essentials_installed: bool) -> &'static str;        // ESSENTIALS_KIT or "Studio Kit"
  ```
  Task 6 must ship exactly these two files, `Grand Piano.sfz` and `Drums.sfz`, at the root of the pack.
- Produces on `Studio`:
  - `pub fn upgrade_starter_track(&mut self) -> bool`. It switches the preset of the only track to `ESSENTIALS_PIANO` and renames the track "Grand Piano". It does this only when the project is still the untouched starter: exactly one track, `preset == "Keys"`, no clips, never saved or loaded, and no edit recorded in the undo history.

- [ ] **Step 1: Write failing tests:**
  - `defaults_follow_essentials`: check both functions with `true` and with `false`;
  - `starter_upgrade_only_when_untouched`: it returns true on a fresh `starter_project()` studio. It returns false after adding a clip, after loading a project, and when the track preset is already `"Bass"`.
- [ ] **Step 2: Run `cargo test defaults starter`.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - `starter_project()` takes `essentials_installed: bool` and uses `default_instrument`.
  - `ui/mod.rs` passes `is_installed(essentials::ID)`. When `poll()` reports that `essentials` was installed, it calls `upgrade_starter_track()` and then `sync()`.
  - Do not change `project::default_kit`, which is the serde default.
- [ ] **Step 4: Run `cargo test`.** Expected: PASS.
- [ ] **Step 5: Commit** with the message `feat: new projects start on the grand piano`.

### Task 5: Sound credits window

**Files:**
- Modify: `src/ui/sound_browser.rs` (a "Sound credits" link at the foot of the Library tab, and a window listing installed packs)

**Interfaces:**
- Consumes: `PackManifest.{name, author, license, homepage}` from Task 2.

- [ ] **Step 1: Write the failing test** `credit_lines_list_every_installed_pack`, against a pure helper `fn credit_lines(packs: &[PackManifest]) -> Vec<String>`. Each line is `"{name} — {author} — {license}"`, with the homepage on its own line when it is not empty.
- [ ] **Step 2: Run `cargo test credit`.** Expected: FAIL.
- [ ] **Step 3: Implement the helper and the window.** Show the homepage as a hyperlink. The built-in synths get the line "Built-in synths and kits — Oxbeat — GPL-3.0-or-later".
- [ ] **Step 4: Run `cargo test`.** Expected: PASS. Then `cargo run` and check that the window opens and its links work.
- [ ] **Step 5: Commit** with the message `feat: sound credits`.

### Task 6: Build tool and the Essentials pack (in `../Oxbeat-Sounds`)

**Files:**
- Modify: `tools/build.mjs`
- Create: `tools/prepare/essentials.mjs`
- Create: `packs/essentials.json`
- Modify: `.github/workflows/publish.yml` (install `flac`)
- Modify: `packs/generaluser-gs.json` (add a `category` to each sound)

**Interfaces:**
- **Recipe format additions:**
  - `"sources": [{ "name": "salamander", "url": "https://…zip" }, …]` is used in place of `source`, and each source is extracted to `<work>/sources/<name>`;
  - `"prepare": "essentials.mjs"` names the preparation script;
  - `"category"` is allowed on the pack and on each sound.
- **The prepare script** exports `default async function prepare({ sources, pack, run })`:
  - `sources` maps each name to an extracted directory;
  - `pack` is the staging directory;
  - `run(cmd, args)` executes a command synchronously.

  Once it has run, `build.mjs` checks that every `sounds[].file` exists, exactly as it does today.
- **`pack.json`** gains `category` and `homepage`, matching Task 2.
- **The pack produces at its root:**
  - `Grand Piano.sfz` and `Drums.sfz`, the file names Task 4 depends on;
  - `GeneralUser-GS.sf2`;
  - `credits/` with every upstream licence file.

- [ ] **Step 1: Test the build tool first.** Add `tools/build.test.mjs`, run with `node --test`. Recipes with `sources` plus `prepare` build a zip from two local `file://` zip fixtures, and `pack.json` contains the categories. Allow the `file://` scheme in tests only, behind `--allow-file`.
- [ ] **Step 2: Run `node --test tools`.** Expected: FAIL.
- [ ] **Step 3: Implement the `build.mjs` changes.** Then make the test pass.
- [ ] **Step 4: Write `packs/essentials.json` and `tools/prepare/essentials.mjs`.** Pin every source to a commit sha.

  | Sound | Source | Reduction | Budget |
  |---|---|---|---|
  | Grand Piano (Keyboards) | sfzinstruments/SalamanderGrandPiano | 5 of 16 velocity layers; 16-bit, 44.1 kHz FLAC; tails trimmed to 10 s with a 200 ms fade; keep release samples only if under budget | ≤ 110 MB |
  | Electric Piano (Keyboards) | sfzinstruments/GregSullivan.E-Pianos | one model | ≤ 20 MB |
  | Upright Piano, Organ (Keyboards) | sgossner/VSCO-2-CE `Keys/` | as is, FLAC | ≤ 25 MB |
  | Violin, Viola, Cello, Double Bass sections, Harp (Strings) | VSCO-2-CE `Strings/` | sustain only | ≤ 40 MB |
  | Flute, Oboe, Clarinet (Winds); Trumpet, Horn, Trombone (Brass) | VSCO-2-CE | sustain only | ≤ 25 MB |
  | Timpani, Glockenspiel, Marimba (Percussion) | VSCO-2-CE `Percussion/` | as is | ≤ 10 MB |
  | Drums (Drums) | sfzinstruments/SamsSonor | as is, under the name `Drums.sfz` | ≤ 35 MB |
  | Harpsichord, guitars, basses, choir, pads | GeneralUser-GS.sf2, bundled with GM presets in `sounds[]` | — | 28 MB |

  For sparse sources, the prepare script uses `git clone --filter=blob:none --sparse` plus `git sparse-checkout set <dirs>` at the pinned sha. The full archive of VSCO-2-CE is 2.3 GB. If VSCO-2-CE has no `.sfz` files, the script writes one per instrument from the sample file names (note name → `pitch_keycenter`, ranges split halfway between neighbouring roots, `v1`/`v2`… → velocity layers). The total is checked at the end, and the script fails if the zip exceeds 250 MB.
- [ ] **Step 5: Run `node tools/build.mjs --only essentials --no-previous --out dist`.** Expected: it prints a zip of ≤ 250 MB.

  Then run the app's real-sounds test against the unpacked pack: `OXBEAT_REAL_SOUNDS=<unpacked dir> cargo test --release loads_real_libraries -- --ignored`. Extend that test (`src/sounds/mod.rs:248`) so that it loads every `.sfz` in the directory, sums `SampleData::bytes()`, and asserts a total under 600 MB.
- [ ] **Step 6: Commit in both repos.** The pack repo gets `feat: Oxbeat Essentials pack`, the app repo `test: memory budget for the essentials pack`. **Do not push.** Ask the user, because pushing `main` publishes the pack.

### Task 7: Store recipes (in `../Oxbeat-Sounds`)

**Files:**
- Create one recipe per pack, each with `category` set:
  - `packs/splendid-grand-piano.json`: Keyboards, public domain;
  - `packs/vsco-strings.json`, `packs/vsco-winds.json`, `packs/vsco-brass.json` and `packs/vsco-percussion.json`: all CC0. They use the same prepare script as Essentials, with every articulation;
  - `packs/musescore-general.json`: MIT. The source is `https://ftp.osuosl.org/pub/musescore/soundfont/MuseScore_General/MuseScore_General.sf2` plus the licence `.md`, and the GM presets are listed like GeneralUser's;
  - `packs/karoryfer-guitars.json`: CC0, made from emilyguitar, shinyguitar and black-and-green-guitars;
  - `packs/karoryfer-basses.json`: CC0, made from black-and-blue-basses, meatbass, growlybass and dsmolken.double-bass;
  - `packs/drumgizmo-drs.json`: CC BY 4.0.
- Modify: `README.md` (the packs table)

- [ ] **Step 1: Confirm the licence of each upstream at its pinned sha** by reading the LICENSE file or the README. Drop any recipe whose licence is not in the allowed list, and note it in the README.
- [ ] **Step 2: Build each recipe** with `node tools/build.mjs --only <id> --no-previous`. Expected: a zip for each, with no error about a missing file.
- [ ] **Step 3: Check that every new pack loads.** Run `OXBEAT_REAL_SOUNDS=<dir> cargo test --release every_real_sfz_plays -- --ignored` on each unpacked pack. Expected: PASS.
- [ ] **Step 4: Commit** with the message `feat: orchestra, guitars, basses, pianos and drums in the store`. Do not push.

### Task 8: Website

**Files:**
- Modify: `site/public/index.html:127-145` (the Sounds section)

- [ ] **Step 1: Rewrite the packs section.**
  - The lead says that Oxbeat downloads its instrument library the first time it opens.
  - The table gains the Essentials row and the Task 7 packs, with their real licences.
  - Remove the "Woodwinds" category wording.
- [ ] **Step 2: Check the page in headless Chrome at 1280 px and at 390 px.** Expected: the table scrolls inside `.table-scroll`, and the page itself does not scroll sideways.
- [ ] **Step 3: Commit** with the message `docs(site): the new instrument library`. Do not deploy and do not push without the user's OK.
