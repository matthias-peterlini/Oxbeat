//! Messages between the UI/MIDI threads and the audio engine.

use std::sync::Arc;

use crate::project::{Note, Project};

pub enum EngineCommand {
    SetProject(Arc<Project>),
    NoteOn {
        track: u64,
        pitch: u8,
        velocity: u8,
    },
    NoteOff {
        track: u64,
        pitch: u8,
    },
    AllNotesOff {
        track: u64,
    },
    Play,
    Stop,
    Record {
        track: u64,
    },
    /// Move the playhead to a beat.
    Locate(f64),
}

pub enum EngineEvent {
    /// Playhead position in beats.
    Position(f64),
    /// A finished recorded note. `note.start_beat` is absolute (song time), not clip-relative.
    RecordedNote { track: u64, note: Note },
    /// Recording stopped; all notes for this take have been sent.
    RecordFinished {
        track: u64,
        start_beat: f64,
        length_beats: f64,
    },
    /// A project snapshot the engine no longer needs; dropped by the receiver so
    /// the audio thread never frees memory.
    Retired(Arc<Project>),
}
