//! SFZ loader for the common opcode subset: regions with key/velocity ranges,
//! tuning, volume, pan, loops, amp envelope, groups/off_by and round robins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::sampler::{LoopMode, Region, SampleData};

type Opcodes = HashMap<String, String>;

/// One `<region>` with all inherited opcodes merged in.
#[derive(Debug, Clone, PartialEq)]
pub struct SfzRegion {
    pub sample: PathBuf,
    pub opcodes: Opcodes,
}

/// Remove `//` line comments and `/* */` block comments.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |i| &r[i + 2..]);
        } else if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |i| &r[i..]);
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Split `a=1 sample=My Piano C4.wav b=2` into pairs. Values may contain spaces:
/// a value runs until the next `name=`.
fn parse_opcodes(s: &str) -> Vec<(String, String)> {
    let bytes = s.as_bytes();
    // Start index of each opcode name and the position of its '='.
    let mut marks = Vec::new();
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'=' {
            continue;
        }
        let mut start = i;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            start -= 1;
        }
        if start < i && (start == 0 || bytes[start - 1].is_ascii_whitespace()) {
            marks.push((start, i));
        }
    }
    let mut out = Vec::new();
    for (k, (start, eq)) in marks.iter().enumerate() {
        let end = marks.get(k + 1).map_or(s.len(), |m| m.0);
        let name = s[*start..*eq].to_string();
        let value = s[eq + 1..end].trim().to_string();
        out.push((name, value));
    }
    out
}

/// Parse SFZ text. `#include` files are resolved relative to `dir`.
pub fn parse(text: &str, dir: &Path) -> Result<Vec<SfzRegion>, String> {
    let mut defines: Vec<(String, String)> = Vec::new();
    let text = expand(text, dir, &mut defines, 0)?;
    let text = strip_comments(&text);

    #[derive(PartialEq, Clone, Copy)]
    enum Level {
        None,
        Control,
        Global,
        Master,
        Group,
        Region,
        Other,
    }
    let mut level = Level::None;
    let (mut control, mut global, mut master, mut group, mut region) = (
        Opcodes::new(),
        Opcodes::new(),
        Opcodes::new(),
        Opcodes::new(),
        Opcodes::new(),
    );
    let mut regions = Vec::new();

    let mut finish_region = |level: Level,
                             control: &Opcodes,
                             global: &Opcodes,
                             master: &Opcodes,
                             group: &Opcodes,
                             region: &Opcodes| {
        if level != Level::Region {
            return;
        }
        let mut merged = global.clone();
        merged.extend(master.clone());
        merged.extend(group.clone());
        merged.extend(region.clone());
        let Some(sample) = merged.get("sample").cloned() else {
            return;
        };
        if sample.starts_with('*') {
            return; // built-in generators (*sine, *noise) aren't supported
        }
        let default_path = control.get("default_path").cloned().unwrap_or_default();
        let rel = format!("{default_path}{sample}").replace('\\', "/");
        for key in ["note_offset", "octave_offset"] {
            if let Some(v) = control.get(key) {
                merged.insert(format!("control_{key}"), v.clone());
            }
        }
        regions.push(SfzRegion {
            sample: dir.join(rel),
            opcodes: merged,
        });
    };

    let mut rest = text.as_str();
    while !rest.is_empty() {
        let (chunk, header) = match rest.find('<') {
            Some(i) => (&rest[..i], true),
            None => (rest, false),
        };
        for (k, v) in parse_opcodes(chunk) {
            let target = match level {
                Level::Control => &mut control,
                Level::Global => &mut global,
                Level::Master => &mut master,
                Level::Group => &mut group,
                Level::Region => &mut region,
                Level::None | Level::Other => continue,
            };
            target.insert(k, v);
        }
        if !header {
            break;
        }
        rest = &rest[chunk.len() + 1..];
        let Some(close) = rest.find('>') else {
            return Err("unclosed header".into());
        };
        let name = rest[..close].trim().to_ascii_lowercase();
        rest = &rest[close + 1..];

        finish_region(level, &control, &global, &master, &group, &region);
        region.clear();
        level = match name.as_str() {
            "control" => {
                control.clear();
                Level::Control
            }
            "global" => {
                global.clear();
                master.clear();
                group.clear();
                Level::Global
            }
            "master" => {
                master.clear();
                group.clear();
                Level::Master
            }
            "group" => {
                group.clear();
                Level::Group
            }
            "region" => Level::Region,
            _ => Level::Other,
        };
    }
    finish_region(level, &control, &global, &master, &group, &region);
    if regions.is_empty() {
        return Err("no playable regions found".into());
    }
    Ok(regions)
}

/// Apply `#define $X value` and `#include "file"` (bounded depth).
fn expand(
    text: &str,
    dir: &Path,
    defines: &mut Vec<(String, String)>,
    depth: u32,
) -> Result<String, String> {
    if depth > 8 {
        return Err("#include nested too deeply".into());
    }
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(def) = t.strip_prefix("#define") {
            let mut parts = def.split_whitespace();
            if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                defines.push((k.to_string(), v.to_string()));
                // Longest names first so $A doesn't clobber $AB.
                defines.sort_by_key(|d| std::cmp::Reverse(d.0.len()));
            }
            continue;
        }
        if let Some(inc) = t.strip_prefix("#include") {
            let name = inc.trim().trim_matches('"');
            let path = dir.join(name.replace('\\', "/"));
            let sub = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            out.push_str(&expand(&sub, dir, defines, depth + 1)?);
            out.push('\n');
            continue;
        }
        let mut l = line.to_string();
        for (k, v) in defines.iter() {
            if l.contains(k.as_str()) {
                l = l.replace(k.as_str(), v);
            }
        }
        out.push_str(&l);
        out.push('\n');
    }
    Ok(out)
}

/// MIDI note from "60", "c4", "C#4", "db3" (SFZ: c4 = 60).
pub fn note_value(s: &str) -> Option<i32> {
    if let Ok(n) = s.trim().parse::<i32>() {
        return Some(n);
    }
    let s = s.trim().to_ascii_lowercase();
    let mut chars = s.chars();
    let base = match chars.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (acc, oct) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b') {
        (-1, r)
    } else {
        (0, rest.as_str())
    };
    let oct: i32 = oct.parse().ok()?;
    Some((oct + 1) * 12 + base + acc)
}

fn num(o: &Opcodes, keys: &[&str], default: f32) -> f32 {
    keys.iter()
        .find_map(|k| o.get(*k).and_then(|v| v.parse::<f32>().ok()))
        .unwrap_or(default)
}

fn note(o: &Opcodes, key: &str, default: i32) -> i32 {
    o.get(key).and_then(|v| note_value(v)).unwrap_or(default)
}

fn clamp_note(n: i32) -> u8 {
    n.clamp(0, 127) as u8
}

/// Build sampler regions, decoding each sample once via `load`.
pub fn build_regions(
    parsed: &[SfzRegion],
    mut load: impl FnMut(&Path) -> Result<Arc<SampleData>, String>,
) -> Result<Vec<Region>, String> {
    let mut out = Vec::new();
    let mut first_error = None;
    for p in parsed {
        let sample = match load(&p.sample) {
            Ok(s) => s,
            Err(e) => {
                first_error.get_or_insert(format!("{}: {e}", p.sample.display()));
                continue;
            }
        };
        let o = &p.opcodes;
        let shift = num(o, &["control_note_offset"], 0.0) as i32
            + 12 * num(o, &["control_octave_offset"], 0.0) as i32;
        let key = o.get("key").and_then(|v| note_value(v));
        let lokey = note(o, "lokey", key.unwrap_or(0)) + shift;
        let hikey = note(o, "hikey", key.unwrap_or(127)) + shift;
        let center = note(o, "pitch_keycenter", key.unwrap_or(60)) + shift;
        let len = sample.len();
        let end = (num(o, &["end"], len as f32) as usize).min(len);
        let loop_mode = match o
            .get("loop_mode")
            .or_else(|| o.get("loopmode"))
            .map(String::as_str)
        {
            Some("one_shot") => LoopMode::OneShot,
            Some("loop_continuous") => LoopMode::Continuous,
            Some("loop_sustain") => LoopMode::Sustain,
            _ => LoopMode::NoLoop,
        };
        if o.get("trigger")
            .is_some_and(|t| t != "attack" && t != "first" && t != "legato")
        {
            continue; // release triggers aren't supported yet
        }
        let mut r = Region::whole(sample, 60);
        r.lokey = clamp_note(lokey);
        r.hikey = clamp_note(hikey);
        r.lovel = num(o, &["lovel"], 0.0).clamp(0.0, 127.0) as u8;
        r.hivel = num(o, &["hivel"], 127.0).clamp(0.0, 127.0) as u8;
        r.key_center = center as f32;
        r.keytrack = num(o, &["pitch_keytrack"], 100.0) / 100.0;
        r.tune = num(o, &["tune", "pitch"], 0.0) / 100.0 + num(o, &["transpose"], 0.0);
        r.gain = 10f32.powf(num(o, &["volume", "gain"], 0.0) / 20.0)
            * num(o, &["amplitude"], 100.0)
            / 100.0;
        r.pan = num(o, &["pan"], 0.0) / 100.0;
        r.veltrack = num(o, &["amp_veltrack"], 100.0) / 100.0;
        r.loop_mode = loop_mode;
        r.loop_start = num(o, &["loop_start", "loopstart"], 0.0) as usize;
        r.loop_end = (num(o, &["loop_end", "loopend"], end as f32) as usize + 1).min(end);
        r.offset = num(o, &["offset"], 0.0) as usize;
        r.end = end;
        r.attack = num(o, &["ampeg_attack"], 0.0);
        r.decay = num(o, &["ampeg_decay"], 0.0);
        r.sustain = num(o, &["ampeg_sustain"], 100.0).clamp(0.0, 100.0) / 100.0;
        r.release = num(o, &["ampeg_release"], 0.03);
        r.group = num(o, &["group"], 0.0).max(0.0) as u32;
        r.off_by = num(o, &["off_by", "offby"], 0.0).max(0.0) as u32;
        r.lorand = num(o, &["lorand"], 0.0);
        r.hirand = num(o, &["hirand"], 1.0);
        r.seq_length = num(o, &["seq_length"], 1.0).max(1.0) as u32;
        r.seq_position = num(o, &["seq_position"], 1.0).max(1.0) as u32;
        out.push(r);
    }
    if out.is_empty() {
        return Err(first_error.unwrap_or_else(|| "no playable regions found".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names() {
        assert_eq!(note_value("c4"), Some(60));
        assert_eq!(note_value("C#4"), Some(61));
        assert_eq!(note_value("db4"), Some(61));
        assert_eq!(note_value("a0"), Some(21));
        assert_eq!(note_value("c-1"), Some(0));
        assert_eq!(note_value("64"), Some(64));
        assert_eq!(note_value("x4"), None);
    }

    #[test]
    fn opcodes_with_spaces_in_values() {
        let o = parse_opcodes("lokey=c4 sample=My Piano C4.wav hikey=62");
        assert_eq!(
            o,
            vec![
                ("lokey".into(), "c4".into()),
                ("sample".into(), "My Piano C4.wav".into()),
                ("hikey".into(), "62".into()),
            ]
        );
    }

    #[test]
    fn inheritance_comments_and_paths() {
        let text = r#"
// a piano
<control> default_path=samples\
#define $VOL -3
<global> volume=$VOL /* everything */ ampeg_release=0.5
<group> lovel=0 hivel=64
<region> sample=p1.wav key=60
<region> sample=p2.wav lokey=61 hikey=72 pitch_keycenter=66 volume=-6
<group> lovel=65
<region> sample=*sine
<region> sample=p3.wav key=d4
"#;
        let r = parse(text, Path::new("/lib")).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].sample, Path::new("/lib").join("samples/p1.wav"));
        assert_eq!(r[0].opcodes["volume"], "-3");
        assert_eq!(r[0].opcodes["hivel"], "64");
        assert_eq!(r[1].opcodes["volume"], "-6");
        assert_eq!(r[2].opcodes["lovel"], "65");
        assert!(!r[2].opcodes.contains_key("hivel"));
    }

    #[test]
    fn builds_regions() {
        let parsed = parse(
            "<region> sample=a.wav lokey=c4 hikey=e4 pitch_keycenter=d4 volume=-6 \
             loop_mode=loop_sustain loop_start=10 loop_end=99 group=2 off_by=3",
            Path::new("/x"),
        )
        .unwrap();
        let regions = build_regions(&parsed, |_| {
            Ok(Arc::new(SampleData {
                rate: 44_100,
                frames: vec![0.0; 400],
            }))
        })
        .unwrap();
        let r = &regions[0];
        assert_eq!((r.lokey, r.hikey, r.key_center), (60, 64, 62.0));
        assert!((r.gain - 0.501).abs() < 0.01);
        assert_eq!(r.loop_mode, LoopMode::Sustain);
        assert_eq!((r.loop_start, r.loop_end), (10, 100));
        assert_eq!((r.group, r.off_by), (2, 3));
    }

    #[test]
    fn missing_samples_report_an_error() {
        let parsed = parse("<region> sample=a.wav", Path::new("/x")).unwrap();
        let err = build_regions(&parsed, |_| Err("not found".into())).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn empty_file_is_an_error() {
        assert!(parse("// nothing", Path::new("/x")).is_err());
    }
}
