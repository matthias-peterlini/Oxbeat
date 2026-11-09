//! The song: tracks, clips and notes. The UI owns this; the engine reads snapshots of it.

pub mod edit;
pub mod effects;
pub mod io;

use serde::{Deserialize, Serialize};

/// Current `project.json` schema version.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub tempo_bpm: f64,
    pub key: String,
    pub tracks: Vec<Track>,
    /// Overall loudness, 0.0..=1.5.
    #[serde(default = "full_volume")]
    pub master_volume: f32,
    #[serde(default)]
    next_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            name: "Untitled".into(),
            tempo_bpm: 120.0,
            key: "C major".into(),
            tracks: Vec::new(),
            master_volume: 1.0,
            next_id: 1,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub kind: TrackKind,
    /// 0.0..=1.5, default 0.8
    pub volume: f32,
    /// -1.0 (left) ..= 1.0 (right)
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// Voice tracks: play the microphone through the track while it is selected.
    #[serde(default)]
    pub monitor: bool,
    /// Effects the track plays through, first to last.
    #[serde(default)]
    pub effects: Vec<effects::Effect>,
    pub clips: Vec<Clip>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TrackKind {
    /// `preset` is a sound id (see `sounds`), e.g. "Keys" or "sfz:/path/piano.sfz".
    Instrument {
        preset: String,
    },
    Audio,
    Drums {
        #[serde(default = "default_kit")]
        kit: String,
    },
}

fn full_volume() -> f32 {
    1.0
}

fn default_kit() -> String {
    "Studio Kit".into()
}

impl Track {
    /// The sound id this track plays through, if it plays notes.
    pub fn sound(&self) -> Option<&str> {
        match &self.kind {
            TrackKind::Instrument { preset } => Some(preset),
            TrackKind::Drums { kit } => Some(kit),
            TrackKind::Audio => None,
        }
    }

    pub fn is_drums(&self) -> bool {
        matches!(self.kind, TrackKind::Drums { .. })
    }

    pub fn is_audio(&self) -> bool {
        matches!(self.kind, TrackKind::Audio)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Clip {
    pub id: u64,
    pub start_beat: f64,
    pub length_beats: f64,
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Audio clips: a WAV file in the project's `audio/` folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// Audio clips: seconds into the file where the clip starts playing.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub audio_offset: f64,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

impl Clip {
    pub fn is_audio(&self) -> bool {
        self.audio.is_some()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub pitch: u8,
    pub velocity: u8,
    /// Relative to the clip start.
    pub start_beat: f64,
    pub length_beats: f64,
}

impl Project {
    /// Returns a fresh id, unique within this project.
    pub fn new_id(&mut self) -> u64 {
        let max_used = self
            .tracks
            .iter()
            .flat_map(|t| std::iter::once(t.id).chain(t.clips.iter().map(|c| c.id)))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_used + 1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_instrument_track(&mut self, preset: &str) -> u64 {
        self.add_track(
            &display_name(preset),
            TrackKind::Instrument {
                preset: preset.to_string(),
            },
        )
    }

    pub fn add_drums_track(&mut self, kit: &str) -> u64 {
        self.add_track(
            "Drums",
            TrackKind::Drums {
                kit: kit.to_string(),
            },
        )
    }

    /// A track for singing or playing a real instrument into the microphone.
    pub fn add_voice_track(&mut self) -> u64 {
        let n = self.tracks.iter().filter(|t| t.is_audio()).count();
        let name = if n == 0 {
            "Voice".to_string()
        } else {
            format!("Voice {}", n + 1)
        };
        let id = self.add_track(&name, TrackKind::Audio);
        if let Some(t) = self.track_mut(id) {
            t.monitor = true;
        }
        id
    }

    /// Audio files the project refers to.
    pub fn audio_files(&self) -> impl Iterator<Item = &str> {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter_map(|c| c.audio.as_deref())
    }

    pub fn add_track(&mut self, name: &str, kind: TrackKind) -> u64 {
        let id = self.new_id();
        self.tracks.push(Track {
            id,
            name: name.to_string(),
            kind,
            volume: 0.8,
            pan: 0.0,
            mute: false,
            solo: false,
            monitor: false,
            effects: Vec::new(),
            clips: Vec::new(),
        });
        id
    }

    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }
}

/// A readable track name from a sound id: "sfz:/x/Grand Piano.sfz" -> "Grand Piano".
pub fn display_name(sound: &str) -> String {
    let s = sound.split('#').next().unwrap_or(sound);
    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let s = s.rsplit_once(':').map_or(s, |(_, r)| r);
    let s = match s.rsplit_once('.') {
        Some((stem, ext)) if ext.len() <= 4 && !stem.is_empty() => stem,
        _ => s,
    };
    let s = s.trim();
    if s.is_empty() {
        "Track".into()
    } else {
        s.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names() {
        assert_eq!(display_name("Keys"), "Keys");
        assert_eq!(display_name("sfz:C:/s/Grand Piano.sfz"), "Grand Piano");
        assert_eq!(display_name("pack:salamander/piano.sfz"), "piano");
        assert_eq!(display_name("sf2:/a/GeneralUser.sf2#0:1"), "GeneralUser");
    }

    #[test]
    fn drums_track_has_kit_sound() {
        let mut p = Project::default();
        let id = p.add_drums_track("808 Kit");
        let t = p.track(id).unwrap();
        assert!(t.is_drums());
        assert_eq!(t.sound(), Some("808 Kit"));
        assert_eq!(t.name, "Drums");
    }

    #[test]
    fn voice_tracks_are_numbered_and_monitored() {
        let mut p = Project::default();
        let a = p.add_voice_track();
        let b = p.add_voice_track();
        assert_eq!(p.track(a).unwrap().name, "Voice");
        assert_eq!(p.track(b).unwrap().name, "Voice 2");
        assert!(p.track(a).unwrap().is_audio() && p.track(a).unwrap().monitor);
        assert_eq!(p.track(a).unwrap().sound(), None);
    }

    #[test]
    fn old_clips_load_without_audio_field() {
        let c: Clip =
            serde_json::from_str(r#"{"id":1,"start_beat":0.0,"length_beats":4.0,"notes":[]}"#)
                .unwrap();
        assert!(!c.is_audio());
        let a: Clip = serde_json::from_str(
            r#"{"id":2,"start_beat":0.0,"length_beats":4.0,"audio":"take-2.wav"}"#,
        )
        .unwrap();
        assert_eq!(a.audio.as_deref(), Some("take-2.wav"));
    }

    #[test]
    fn track_ids_unique() {
        let mut p = Project::default();
        let a = p.add_instrument_track("Keys");
        let b = p.add_instrument_track("Keys");
        assert_ne!(a, b);
    }

    #[test]
    fn ids_stay_unique_after_load_without_counter() {
        let mut p = Project::default();
        p.add_instrument_track("Keys");
        p.next_id = 0; // e.g. hand-edited file
        let b = p.add_instrument_track("Bass");
        assert_ne!(p.tracks[0].id, b);
    }
}
