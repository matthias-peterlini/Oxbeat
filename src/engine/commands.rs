//! Messages between the UI/MIDI threads and the audio engine, and the queues
//! that carry them.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::instrument::Instrument;
use super::sampler::SampleData;
use crate::project::{Note, Project};

const COMMAND_QUEUE: usize = 4096;
const EVENT_QUEUE: usize = 16_384;
/// Microphone samples on their way to the take writer: about 20 s at 48 kHz, so a
/// busy disk or a stalled thread never loses audio.
const RECORDED_QUEUE: usize = 1 << 20;

pub enum EngineCommand {
    SetProject(Arc<Project>),
    /// Replace a track's instrument. `sound` is `sound_hash` of its sound id.
    SetInstrument {
        track: u64,
        sound: u64,
        instrument: Box<Instrument>,
    },
    /// Attach (or with `None`, forget) the decoded audio of an audio clip.
    SetAudio {
        clip: u64,
        data: Option<Arc<SampleData>>,
    },
    /// Microphone samples (mono, at the engine's sample rate), or `None` when the
    /// microphone is closed.
    SetInput(Option<rtrb::Consumer<f32>>),
    /// The track that live input (keys, MIDI, microphone) goes to.
    SetArmed(u64),
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
    /// Recording stopped; all notes (or microphone samples) for this take have been sent.
    RecordFinished {
        track: u64,
        start_beat: f64,
        length_beats: f64,
    },
    /// A project snapshot the engine no longer needs; dropped by the receiver so
    /// the audio thread never frees memory.
    Retired(Arc<Project>),
    /// An instrument the engine no longer needs, same reason.
    RetiredInstrument(Box<Instrument>),
    /// Clip audio the engine no longer needs, same reason.
    RetiredAudio(Arc<SampleData>),
    /// A microphone queue the engine no longer reads, same reason.
    RetiredInput(rtrb::Consumer<f32>),
}

/// Levels the engine measures for the UI to draw.
#[derive(Default)]
pub struct Meters {
    input: AtomicU32,
}

impl Meters {
    /// Loudest microphone sample since the last call (0.0..=1.0+).
    pub fn take_input(&self) -> f32 {
        f32::from_bits(self.input.swap(0, Ordering::Relaxed))
    }

    pub(super) fn raise_input(&self, peak: f32) {
        // Bit patterns of non-negative floats sort like the floats themselves.
        self.input
            .fetch_max(peak.abs().to_bits(), Ordering::Relaxed);
    }
}

/// The audio thread's side of the link.
pub struct EngineEnd {
    pub commands: rtrb::Consumer<EngineCommand>,
    pub events: rtrb::Producer<EngineEvent>,
    /// Microphone samples captured while recording a voice track.
    pub recorded: rtrb::Producer<f32>,
    pub meters: Arc<Meters>,
}

/// The UI's side of the link.
pub struct UiEnd {
    pub commands: rtrb::Producer<EngineCommand>,
    pub events: rtrb::Consumer<EngineEvent>,
    pub recorded: rtrb::Consumer<f32>,
    pub meters: Arc<Meters>,
}

/// Fresh queues for linking the UI to an engine.
pub fn link() -> (UiEnd, EngineEnd) {
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::new(COMMAND_QUEUE);
    let (evt_tx, evt_rx) = rtrb::RingBuffer::new(EVENT_QUEUE);
    let (rec_tx, rec_rx) = rtrb::RingBuffer::new(RECORDED_QUEUE);
    let meters = Arc::new(Meters::default());
    (
        UiEnd {
            commands: cmd_tx,
            events: evt_rx,
            recorded: rec_rx,
            meters: meters.clone(),
        },
        EngineEnd {
            commands: cmd_rx,
            events: evt_tx,
            recorded: rec_tx,
            meters,
        },
    )
}
