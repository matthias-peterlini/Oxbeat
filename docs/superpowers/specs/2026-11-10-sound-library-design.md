# Sound library — design

## Why

Oxbeat wants to be an open-source alternative to GarageBand. The engine can
already play real sampled instruments (SF2, SFZ, audio files) and download
packs from the Store, but out of the box it only has four synth tones and two
synth kits. A new "Keys" track is a soft electric-piano synth, which makes the
first impression feel like a toy. GarageBand's strength is that a new user
picks "Piano", "Strings" or "Guitar" and hears a real instrument immediately.

## Goals

- A new Keys track plays a **real grand piano** once the library is installed.
- A **core library** ("Oxbeat Essentials") covering the instruments people
  reach for first: grand and upright piano, electric piano, harpsichord,
  organ, strings (violin, viola, cello, double bass, harp), acoustic and
  electric guitars, bass, a few winds and brass, orchestral percussion, and
  acoustic drum kits.
- The core library is **downloaded automatically on first launch** (decided
  by the project owner), in the background, using the existing pack
  machinery (`src/sounds/packs.rs`, the `packs` release of Oxbeat-beats).
- The **Store grows** from 5 packs to a curated catalogue grouped by category.
- After the download, everything works **offline**.

## Non-goals

- Recording our own samples.
- Streaming samples from disk (only if the memory work below is not enough).
- Matching GarageBand sound-for-sound. Piano and orchestra can get close;
  free guitars are noticeably weaker and that is accepted.

## Anti-goals (a failure even if it "works")

- The app is unusable until a download finishes. The window must open at
  once and the synths must keep working while the library downloads, or
  forever if the user is offline.
- Any sample with a non-commercial, "Sampling Plus" or unstated licence.
  Users must be free to sell music made with Oxbeat.
- Loading one piano eats gigabytes of RAM.
- An existing project changes sound after the update. Projects that say
  `"Keys"` keep the synth; only new tracks get the piano.

## Licences checked (2026-11-10)

Only CC0, public domain, CC BY, CC BY-SA and MIT are allowed. CC BY and
CC BY-SA need credit, which the app shows (see Credits).

| Library | Instruments | Licence | Source |
|---|---|---|---|
| Salamander Grand Piano | Yamaha C5 grand, 16 velocity layers | CC BY 3.0 | sfzinstruments/SalamanderGrandPiano |
| Splendid Grand Piano | Steinway, 4 layers (256 MB) | Public domain (Akai) | sfzinstruments/SplendidGrandPiano |
| VSCO 2 Community Edition | Strings, woodwinds, brass, orchestral percussion, upright piano, organ | CC0 | sgossner/VSCO-2-CE |
| MuseScore General | Full General MIDI bank, 215 MB SF2 | MIT (credit notice required) | ftp.osuosl.org/pub/musescore/soundfont |
| GeneralUser GS (already a pack) | GM bank incl. harpsichord, guitars | GeneralUser GS v2.0 | mrbumpy409/GeneralUser-GS |
| Greg Sullivan E-Pianos | Electric pianos | CC BY 3.0 | sfzinstruments/GregSullivan.E-Pianos |
| Karoryfer libraries | Emilyguitar, Shinyguitar, Black and Green Guitars, Black and Blue Basses, Meatbass, Growlybass, Bigcat Cello, Big Rusty Drums, Swirly Drums, Unruly Drums, and more | CC0 | sfzinstruments/karoryfer.* |
| Double Bass (dsmolken) | Upright bass | CC0 | sfzinstruments/dsmolken.double-bass |
| DrumGizmo DRS / Muldjord kits | Acoustic drum kits | CC BY 4.0 | sfzinstruments/DrumGizmo.* |
| Naked Drums | Acoustic drums | CC BY 4.0 | sfzinstruments/WilkinsonAudio.NakedDrums |
| Headroom Piano | Piano | CC BY 4.0 | sfzinstruments/BengtNilsson.HeadroomPiano |
| Osiris Piano, Hungarian Zither, Erhu, Ganjo, Virtuosity Drums | Assorted | CC0 | sfzinstruments/* |

Excluded:
- G-Town Church organ: CC Sampling Plus.
- Clavecin harpsichord: no licence stated. The harpsichord comes from the GM banks instead.
- jRhodes3, Maestro Concert Grand, Ovation Guitar, Terkelsen Marimba and Mandolin: the licence was not confirmed. Check each one before it is used.

## Oxbeat Essentials (the first-launch pack)

Target download: **≤ 250 MB**. It is a normal pack in Oxbeat-beats with the id
`essentials`, built by `tools/build.mjs` from several upstream sources. The
proposed contents are below; the builder settles exact cuts by ear and by size:

- **Grand Piano:** Salamander. Reduce it to about 5 velocity layers, keep
  every third semitone, and store it as FLAC. Release samples stay in only
  if they fit the size budget.
- **Upright Piano** and **Organ:** VSCO 2 CE.
- **Electric Piano:** Greg Sullivan E-Pianos, one model.
- **Harpsichord, Acoustic and Electric Guitar, Clean Bass, Choir, Synth
  pads:** the matching presets of GeneralUser GS. Essentials depends on that
  pack, or bundles it; it is only 28 MB.
- **Strings:** VSCO 2 CE violin, viola, cello and double-bass sections, plus
  harp.
- **Winds and Brass:** VSCO 2 CE flute, clarinet, oboe, trumpet, French horn
  and trombone, one articulation each (sustain).
- **Orchestral percussion:** VSCO 2 CE timpani, glockenspiel and marimba.
- **Drums:** one real acoustic kit, either Sam's Sonor (already packed) or a
  DrumGizmo kit, reduced.

VSCO 2 CE ships raw samples. If no SFZ mapping comes with it, the recipe
writes one. The recipe format already has a place for this: "the sounds
Oxbeat shows, with names, captions and presets".

## First-launch flow

1. The app opens normally and the synths work.
2. If Essentials is not installed and the app has not yet downloaded it,
   the download starts in the background. A slim, dismissible strip shows:
   "Downloading the instrument library… 42% (95 of 230 MB)".
3. With no connection, the strip says so and the app retries on the next
   launch. The sound browser also offers an "Install instruments" button.
4. When the download finishes, the sound browser shows the new instruments
   at once, without a restart.
5. A user can remove Essentials from the Store like any other pack. Once
   they have removed it, the app does not download it again by itself.

## Defaults

- **Add Track → Keys** (and the starter track of a new project) uses
  `Essentials / Grand Piano` when it is installed, and falls back to the
  `"Keys"` synth otherwise.
- **The default drum kit** uses the Essentials acoustic kit when it is installed.
- **Saved projects keep the sound id they already store.** Nothing changes
  for them.

## Sound browser

Sounds are grouped by **instrument category**, not by pack: Keyboards,
Strings, Guitars, Bass, Winds, Brass, Percussion, Drums, Synths, World. Each
sound in a pack manifest gets a `category` field. Today the category exists
only per pack in the index. Built-in synths go under Synths, and imported
user files keep their current place.

## Store

- Add packs from the licence table, one recipe each, grouped by the same
  categories.
- Each entry shows its size and licence, as it does today.
- Order of work: pianos (Salamander full, Splendid), the full VSCO orchestra,
  MuseScore General, the Karoryfer guitars and basses, then the drum kits.

## Engine: memory

`SampleData` keeps every sample as interleaved stereo `f32`, 8 bytes per frame.
The full Salamander library at 48 kHz would take several GB of RAM. Required
changes:

- **Keep mono samples mono and store them as `i16`.** For a mono library
  that cuts memory by 4×. Convert to float when mixing. Most libraries above
  are mono or 16-bit.
- **Load a sampler's samples once and share them across tracks.** The cache
  in `src/sounds/mod.rs` already shares SoundFonts and decoded samples
  through `Weak`, so check that this also holds for SFZ regions.
- **Budget:** Essentials fully loaded should stay under about 600 MB of RAM.
  Only if the full Store pianos cannot fit should disk streaming come back
  as a separate project.

## Credits

There is a "Sound credits" screen, reached from About or the Store. It lists
each installed pack with its author, licence and link, taken from the pack
manifest. This covers attribution for CC BY and CC BY-SA, and the MIT notice
for MuseScore General. The website's Sounds table lists the same packs.

## Left to the builder

- Exact sample reductions, and loudness matching between instruments.
- Which VSCO articulations to use beyond sustain.
- The wording of the download strip, and where the "Install instruments"
  button goes.
- Whether Essentials bundles GeneralUser GS or depends on it.
