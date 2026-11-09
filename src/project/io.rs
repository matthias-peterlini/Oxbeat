//! Save/load a project bundle: a folder `<Name>.oxbeat/` containing `project.json`
//! and an `audio/` folder with its recordings.

use std::path::Path;

use super::{Project, SCHEMA_VERSION};

pub const PROJECT_FILE: &str = "project.json";
/// Folder inside the bundle that holds recorded and imported audio.
pub const AUDIO_DIR: &str = "audio";

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("Could not read the project: {0}")]
    Io(#[from] std::io::Error),
    #[error("The project file is damaged: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(
        "This project was made with a newer version of Oxbeat (format {found}). Please update Oxbeat."
    )]
    TooNew { found: u32 },
}

pub fn save(project: &Project, bundle_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(bundle_dir)?;
    let json = serde_json::to_string_pretty(project)?;
    // Write to a temp file first so a crash mid-save never destroys the old project.
    let tmp = bundle_dir.join("project.json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, bundle_dir.join(PROJECT_FILE))?;
    Ok(())
}

pub fn load(bundle_dir: &Path) -> Result<Project, LoadError> {
    let text = std::fs::read_to_string(bundle_dir.join(PROJECT_FILE))?;
    // Check the version before parsing the full schema so newer files get a clear message.
    let header: serde_json::Value = serde_json::from_str(&text)?;
    let found = header.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if found > SCHEMA_VERSION {
        return Err(LoadError::TooNew { found });
    }
    Ok(serde_json::from_value(header)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Clip, Note};

    fn sample() -> Project {
        let mut p = Project::default();
        let t = p.add_instrument_track("Keys");
        let clip_id = p.new_id();
        p.track_mut(t).unwrap().clips.push(Clip {
            id: clip_id,
            start_beat: 4.0,
            length_beats: 4.0,
            notes: vec![
                Note {
                    pitch: 60,
                    velocity: 100,
                    start_beat: 0.0,
                    length_beats: 1.0,
                },
                Note {
                    pitch: 64,
                    velocity: 90,
                    start_beat: 1.5,
                    length_beats: 0.5,
                },
            ],
            audio: None,
        });
        p
    }

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.oxbeat");
        let p = sample();
        save(&p, &bundle).unwrap();
        assert_eq!(load(&bundle).unwrap(), p);
    }

    #[test]
    fn rejects_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(PROJECT_FILE),
            r#"{"version":2,"name":"x","tempo_bpm":120.0,"key":"C major","tracks":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            load(dir.path()),
            Err(LoadError::TooNew { found: 2 })
        ));
    }

    #[test]
    fn broken_json_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PROJECT_FILE), "{oops").unwrap();
        assert!(matches!(load(dir.path()), Err(LoadError::Parse(_))));
    }

    #[test]
    fn missing_bundle_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load(&dir.path().join("nope")),
            Err(LoadError::Io(_))
        ));
    }
}
