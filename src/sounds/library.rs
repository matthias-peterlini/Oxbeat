//! What sounds are on this computer: downloaded packs and the user's imports.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Preset, file_sound};

/// Oxbeat's data folder (packs, library list).
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Oxbeat")
}

pub fn packs_dir() -> PathBuf {
    data_dir().join("packs")
}

/// Pack ids become folder names: keep them boring.
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SoundKind {
    #[default]
    Instrument,
    Drums,
}

/// A sound inside a pack.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackSound {
    pub name: String,
    /// Path inside the pack folder.
    pub file: String,
    #[serde(default)]
    pub kind: SoundKind,
    #[serde(default)]
    pub caption: String,
    /// "bank:program" for SoundFonts; none means "let me choose".
    #[serde(default)]
    pub preset: Option<String>,
}

/// `pack.json` at the root of every pack.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PackManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    pub sounds: Vec<PackSound>,
}

impl PackSound {
    /// The sound id, or `None` for a SoundFont that still needs a preset picked.
    pub fn sound_id(&self, pack: &str) -> Option<String> {
        let mut id = format!("pack:{pack}/{}", self.file);
        match &self.preset {
            Some(p) => {
                id.push('#');
                id.push_str(p);
            }
            None if self.file.to_ascii_lowercase().ends_with(".sf2") => return None,
            None => {}
        }
        Some(id)
    }

    pub fn path(&self, pack: &str) -> PathBuf {
        packs_dir().join(pack).join(&self.file)
    }
}

pub fn read_manifest(dir: &Path) -> Result<PackManifest, String> {
    let text = std::fs::read_to_string(dir.join("pack.json")).map_err(|e| e.to_string())?;
    let m: PackManifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if !is_safe_id(&m.id) {
        return Err(format!("bad pack id {:?}", m.id));
    }
    Ok(m)
}

/// Packs installed in `packs_dir()`, by name.
pub fn installed_packs() -> Vec<PackManifest> {
    installed_in(&packs_dir())
}

pub fn installed_in(dir: &Path) -> Vec<PackManifest> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut packs: Vec<PackManifest> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| read_manifest(&e.path()).ok())
        .collect();
    packs.sort_by_key(|p| p.name.to_lowercase());
    packs
}

/// A file the user imported, remembered between sessions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Imported {
    pub name: String,
    pub sound: String,
    pub kind: SoundKind,
}

fn imports_file() -> PathBuf {
    data_dir().join("library.json")
}

pub fn imports() -> Vec<Imported> {
    std::fs::read_to_string(imports_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Remember an import (most recent first, no duplicates).
pub fn add_import(item: Imported) {
    let mut list = imports();
    list.retain(|i| i.sound != item.sound);
    list.insert(0, item);
    list.truncate(200);
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(text) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(imports_file(), text);
    }
}

pub fn import_sound(path: &Path, name: &str, preset: Option<Preset>, kind: SoundKind) -> String {
    let sound = file_sound(path, preset);
    add_import(Imported {
        name: name.to_string(),
        sound: sound.clone(),
        kind,
    });
    sound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_ids() {
        assert!(is_safe_id("salamander-piano_2"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("../x"));
        assert!(!is_safe_id("Piano"));
        assert!(!is_safe_id("a/b"));
    }

    #[test]
    fn sound_ids_from_pack() {
        let s = PackSound {
            name: "Piano".into(),
            file: "GeneralUser.sf2".into(),
            kind: SoundKind::Instrument,
            caption: String::new(),
            preset: Some("0:0".into()),
        };
        assert_eq!(
            s.sound_id("gm").as_deref(),
            Some("pack:gm/GeneralUser.sf2#0:0")
        );
        let any = PackSound {
            preset: None,
            ..s.clone()
        };
        assert_eq!(any.sound_id("gm"), None);
        let sfz = PackSound {
            file: "piano.sfz".into(),
            preset: None,
            ..s
        };
        assert_eq!(sfz.sound_id("p").as_deref(), Some("pack:p/piano.sfz"));
    }

    #[test]
    fn lists_installed_packs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("zed");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("pack.json"),
            r#"{"id":"zed","name":"Zed Pack","sounds":[{"name":"Hit","file":"hit.wav","kind":"drums"}]}"#,
        )
        .unwrap();
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("pack.json"), "{").unwrap();
        let packs = installed_in(dir.path());
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].sounds[0].kind, SoundKind::Drums);
    }
}
