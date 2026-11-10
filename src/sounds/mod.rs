//! Turning sound ids into playable instruments: built-ins, imported files, packs.
//!
//! Sound ids:
//! - `Keys`, `Studio Kit`, … built-ins (optionally `builtin:` prefixed)
//! - `file:<path>[#bank:program]` an imported SF2, SFZ or audio file
//! - `pack:<pack-id>/<file in pack>[#bank:program]` a sound from a downloaded pack

pub mod decode;
pub mod essentials;
pub mod library;
pub mod packs;
pub mod sfz;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use crate::engine::instrument::{Instrument, SoundFontInstrument};
use crate::engine::sampler::{Region, SampleData, Sampler};

/// Built-in instruments: (sound id, caption).
pub const BUILTIN_INSTRUMENTS: [(&str, &str); 4] = [
    ("Keys", "Soft electric piano"),
    ("Bass", "Warm, round synth bass"),
    ("Lead", "Bright square lead"),
    ("Pad", "Slow, wide synth pad"),
];

/// Built-in drum kits: (sound id, caption).
pub const BUILTIN_KITS: [(&str, &str); 2] = [
    ("Studio Kit", "Punchy acoustic-style kit"),
    ("808 Kit", "Deep electronic kick and crisp hats"),
];

/// The Essentials pack's grand piano and drum kit (the pack ships exactly these files).
pub const ESSENTIALS_PIANO: &str = "pack:essentials/Grand Piano.sfz";
pub const ESSENTIALS_KIT: &str = "pack:essentials/Drums.sfz";

/// The instrument new projects start on.
pub fn default_instrument(essentials_installed: bool) -> &'static str {
    if essentials_installed {
        ESSENTIALS_PIANO
    } else {
        "Keys"
    }
}

/// The drum kit new drum tracks start on.
pub fn default_kit(essentials_installed: bool) -> &'static str {
    if essentials_installed {
        ESSENTIALS_KIT
    } else {
        "Studio Kit"
    }
}

/// File types that can be imported as sounds.
pub const IMPORT_EXTENSIONS: [&str; 7] = ["sf2", "sfz", "wav", "flac", "ogg", "mp3", "aif"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    pub bank: i32,
    pub program: i32,
}

impl Preset {
    /// SoundFont bank 128 holds drum kits.
    pub fn is_drums(&self) -> bool {
        self.bank == 128
    }
}

/// Where a sound id's data lives.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub path: PathBuf,
    pub preset: Option<Preset>,
}

pub fn file_sound(path: &Path, preset: Option<Preset>) -> String {
    let mut s = format!("file:{}", path.display());
    if let Some(p) = preset {
        s.push_str(&format!("#{}:{}", p.bank, p.program));
    }
    s
}

pub fn resolve(sound: &str) -> Result<Resolved, String> {
    let (body, preset) = match sound.rsplit_once('#') {
        Some((b, frag)) => {
            let (bank, prog) = frag.split_once(':').ok_or("bad preset")?;
            let preset = Preset {
                bank: bank.parse().map_err(|_| "bad preset bank")?,
                program: prog.parse().map_err(|_| "bad preset program")?,
            };
            (b, Some(preset))
        }
        None => (sound, None),
    };
    let path = if let Some(p) = body.strip_prefix("file:") {
        PathBuf::from(p)
    } else if let Some(p) = body.strip_prefix("pack:") {
        let (id, rel) = p.split_once('/').ok_or("bad pack sound")?;
        if !library::is_safe_id(id) || rel.split(['/', '\\']).any(|c| c == "..") {
            return Err("bad pack sound".into());
        }
        library::packs_dir().join(id).join(rel)
    } else {
        return Err("unknown sound".into());
    };
    Ok(Resolved { path, preset })
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Loaded data, shared between tracks and kept only while some track uses it.
#[derive(Default)]
struct Cache {
    samples: HashMap<PathBuf, Weak<SampleData>>,
    fonts: HashMap<PathBuf, Weak<rustysynth::SoundFont>>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Cache::default))
}

fn sample(path: &Path) -> Result<Arc<SampleData>, String> {
    if let Some(s) = with_cache(|c| c.samples.get(path).and_then(Weak::upgrade)) {
        return Ok(s);
    }
    let s = Arc::new(decode::decode_compact(path)?);
    with_cache(|c| c.samples.insert(path.to_path_buf(), Arc::downgrade(&s)));
    Ok(s)
}

pub fn soundfont(path: &Path) -> Result<Arc<rustysynth::SoundFont>, String> {
    if let Some(f) = with_cache(|c| c.fonts.get(path).and_then(Weak::upgrade)) {
        return Ok(f);
    }
    let mut file = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    let f = Arc::new(
        rustysynth::SoundFont::new(&mut file)
            .map_err(|e| format!("not a valid SoundFont ({e:?})"))?,
    );
    with_cache(|c| c.fonts.insert(path.to_path_buf(), Arc::downgrade(&f)));
    Ok(f)
}

/// Every preset in a SoundFont: (name, preset), sorted by bank then program.
pub fn soundfont_presets(path: &Path) -> Result<Vec<(String, Preset)>, String> {
    let f = soundfont(path)?;
    let mut v: Vec<(String, Preset)> = f
        .get_presets()
        .iter()
        .map(|p| {
            (
                p.get_name().trim().to_string(),
                Preset {
                    bank: p.get_bank_number(),
                    program: p.get_patch_number(),
                },
            )
        })
        .collect();
    v.sort_by_key(|(_, p)| (p.bank, p.program));
    Ok(v)
}

fn load_soundfont(path: &Path, preset: Option<Preset>, sr: u32) -> Result<Instrument, String> {
    let font = soundfont(path)?;
    let mut settings = rustysynth::SynthesizerSettings::new(sr as i32);
    settings.enable_reverb_and_chorus = false;
    let mut synth = rustysynth::Synthesizer::new(&font, &settings).map_err(|e| format!("{e:?}"))?;
    // rustysynth defaults to 0.5, quieter than our other instruments.
    synth.set_master_volume(1.0);
    let preset = preset.unwrap_or(Preset {
        bank: 0,
        program: 0,
    });
    let channel = if preset.is_drums() { 9 } else { 0 };
    if !preset.is_drums() {
        synth.process_midi_message(channel, 0xB0, 0x00, preset.bank);
    }
    synth.process_midi_message(channel, 0xC0, preset.program, 0);
    Ok(Instrument::SoundFont(Box::new(SoundFontInstrument {
        synth,
        channel,
    })))
}

fn load_sfz(path: &Path, sr: u32) -> Result<Instrument, String> {
    let text = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&text);
    let dir = path.parent().unwrap_or(Path::new("."));
    let parsed = sfz::parse(&text, dir)?;
    let regions = sfz::build_regions(&parsed, sample)?;
    Ok(Instrument::Sampler(Sampler::new(Arc::new(regions), sr)))
}

fn load_audio(path: &Path, sr: u32) -> Result<Instrument, String> {
    let s = sample(path)?;
    Ok(Instrument::Sampler(Sampler::new(
        Arc::new(vec![Region::whole(s, 60)]),
        sr,
    )))
}

/// Build the instrument for a sound id at the engine's sample rate.
pub fn load(sound: &str, sample_rate: u32) -> Result<Instrument, String> {
    if let Some(b) = Instrument::builtin(sound, sample_rate) {
        return Ok(b);
    }
    let r = resolve(sound)?;
    if !r.path.exists() {
        return Err(format!("{} is missing", r.path.display()));
    }
    match ext(&r.path).as_str() {
        "sf2" => load_soundfont(&r.path, r.preset, sample_rate),
        "sfz" => load_sfz(&r.path, sample_rate),
        _ => load_audio(&r.path, sample_rate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_essentials() {
        assert_eq!(default_instrument(true), "pack:essentials/Grand Piano.sfz");
        assert_eq!(default_instrument(false), "Keys");
        assert_eq!(default_kit(true), "pack:essentials/Drums.sfz");
        assert_eq!(default_kit(false), "Studio Kit");
    }

    #[test]
    fn every_builtin_loads() {
        for (id, _) in BUILTIN_INSTRUMENTS.iter().chain(&BUILTIN_KITS) {
            assert!(load(id, 48_000).is_ok(), "{id}");
        }
        assert!(load("nope", 48_000).is_err());
    }

    #[test]
    fn resolves_ids() {
        let r = resolve("file:/x/GeneralUser GS.sf2#128:0").unwrap();
        assert_eq!(r.path, PathBuf::from("/x/GeneralUser GS.sf2"));
        assert_eq!(
            r.preset,
            Some(Preset {
                bank: 128,
                program: 0
            })
        );
        let r = resolve("pack:piano/sfz/piano.sfz").unwrap();
        assert!(r.path.ends_with("packs/piano/sfz/piano.sfz") || r.path.ends_with("piano.sfz"));
        assert!(resolve("pack:../../etc/x.sfz").is_err());
        assert!(resolve("pack:p/../../x.sfz").is_err());
        assert_eq!(
            file_sound(
                Path::new("/a.sf2"),
                Some(Preset {
                    bank: 0,
                    program: 5
                })
            ),
            "file:/a.sf2#0:5"
        );
    }

    /// Loads the unpacked Oxbeat Essentials pack from `$OXBEAT_REAL_SOUNDS`, plays
    /// its defaults, and checks that all of its SFZ samples fit in 600 MB of RAM.
    /// Run with `cargo test --release loads_real_libraries -- --ignored`.
    #[test]
    #[ignore]
    fn loads_real_libraries() {
        let dir = PathBuf::from(std::env::var("OXBEAT_REAL_SOUNDS").unwrap());
        let gu = dir.join("GeneralUser-GS.sf2");
        let presets = soundfont_presets(&gu).unwrap();
        for (bank, program) in [
            (0, 0),
            (0, 4),
            (0, 24),
            (0, 33),
            (0, 48),
            (128, 0),
            (128, 25),
        ] {
            let p = presets
                .iter()
                .find(|(_, p)| p.bank == bank && p.program == program)
                .unwrap_or_else(|| panic!("no preset {bank}:{program}"));
            println!("{bank}:{program} = {}", p.0);
        }
        let ids = [
            (
                file_sound(
                    &gu,
                    Some(Preset {
                        bank: 0,
                        program: 0,
                    }),
                ),
                60,
            ),
            (
                file_sound(
                    &gu,
                    Some(Preset {
                        bank: 128,
                        program: 0,
                    }),
                ),
                38,
            ),
            (file_sound(&dir.join("Grand Piano.sfz"), None), 60),
            (file_sound(&dir.join("Drums.sfz"), None), 38),
        ];
        for (id, note) in ids {
            let t = std::time::Instant::now();
            let mut inst = load(&id, 48_000).unwrap_or_else(|e| panic!("{id}: {e}"));
            println!("{id} loaded in {:?}", t.elapsed());
            inst.note_on(note, 110);
            let (mut l, mut r) = (vec![0.0; 9600], vec![0.0; 9600]);
            inst.render(&mut l, &mut r);
            let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
            println!("  peak {peak}");
            assert!(peak > 0.01, "{id} silent");
        }

        // Every SFZ in the pack, decoded, must fit the memory budget. Samples are
        // shared through the cache, so each one is counted once.
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| ext(p) == "sfz")
            .collect();
        files.sort();
        assert!(files.len() >= 2, "no .sfz files in {}", dir.display());
        let mut seen = std::collections::HashSet::new();
        let mut total = 0usize;
        let mut keep = Vec::new();
        for f in &files {
            let text = std::fs::read_to_string(f).unwrap();
            let parsed = sfz::parse(&text, &dir).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let regions = sfz::build_regions(&parsed, sample)
                .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let mut own = 0usize;
            for r in &regions {
                if seen.insert(Arc::as_ptr(&r.sample) as usize) {
                    own += r.sample.bytes();
                }
            }
            total += own;
            println!("{:7.1} MB  {}", own as f64 / 1048576.0, f.display());
            keep.push(regions); // hold the samples so the cache can't drop them
        }
        println!("{:7.1} MB  total", total as f64 / 1048576.0);
        assert!(total < 600 * 1048576, "{total} bytes of samples");
    }

    /// Every .sfz under `$OXBEAT_REAL_SOUNDS` must load and make sound at some key.
    #[test]
    #[ignore]
    fn every_real_sfz_plays() {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if ext(&p) == "sfz" {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            Path::new(&std::env::var("OXBEAT_REAL_SOUNDS").unwrap()),
            &mut files,
        );
        let mut failures = Vec::new();
        for f in files {
            let id = file_sound(&f, None);
            match load(&id, 48_000) {
                Err(e) => failures.push(format!("{}: {e}", f.display())),
                Ok(mut inst) => {
                    for n in [36, 48, 60, 72] {
                        inst.note_on(n, 100);
                    }
                    let (mut l, mut r) = (vec![0.0; 9600], vec![0.0; 9600]);
                    inst.render(&mut l, &mut r);
                    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
                    println!("{peak:.3}  {}", f.display());
                    if peak < 0.005 {
                        failures.push(format!("{}: silent", f.display()));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn loads_wav_and_sfz_files() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..4800 {
            w.write_sample(((i as f32 * 0.05).sin() * 20_000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
        std::fs::write(
            dir.path().join("inst.sfz"),
            "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=60",
        )
        .unwrap();

        for id in [
            file_sound(&wav, None),
            file_sound(&dir.path().join("inst.sfz"), None),
        ] {
            let mut inst = load(&id, 48_000).unwrap();
            inst.note_on(60, 120);
            let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
            inst.render(&mut l, &mut r);
            assert!(l.iter().any(|x| x.abs() > 0.05), "{id} silent");
        }
        assert!(load(&file_sound(&dir.path().join("gone.wav"), None), 48_000).is_err());
    }

    /// Installs `$OXBEAT_PACK_ZIP` (a zip built by the sounds repo) like the Store
    /// does, then plays every sound its pack.json lists.
    #[test]
    #[ignore]
    fn built_pack_installs_and_plays() {
        let zip = PathBuf::from(std::env::var("OXBEAT_PACK_ZIP").unwrap());
        let id = zip.file_name().unwrap().to_string_lossy();
        let id = id.rsplit_once('-').unwrap().0.to_string();
        std::fs::create_dir_all(library::packs_dir()).unwrap();
        crate::sounds::packs::extract_pack(&zip, &library::packs_dir(), &id).unwrap();
        let pack = library::read_manifest(&library::packs_dir().join(&id)).unwrap();
        assert!(!pack.sounds.is_empty());
        for s in &pack.sounds {
            let Some(sound) = s.sound_id(&pack.id) else {
                continue;
            };
            let mut inst = load(&sound, 48_000).unwrap_or_else(|e| panic!("{sound}: {e}"));
            for n in [36, 38, 42, 48, 60, 72] {
                inst.note_on(n, 100);
            }
            let (mut l, mut r) = (vec![0.0; 9600], vec![0.0; 9600]);
            inst.render(&mut l, &mut r);
            let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
            println!("{peak:.3}  {}", s.name);
            assert!(peak > 0.005, "{} is silent", s.name);
        }
    }
}
