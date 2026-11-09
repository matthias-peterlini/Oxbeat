//! Non-visual app state: the project, the engine link, recording assembly.
//! Kept free of egui so it can be unit-tested.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::engine::commands::{EngineCommand, EngineEvent};
use crate::engine::instrument::{Instrument, sound_hash};
use crate::project::{Clip, Note, Project};

const COMMAND_QUEUE: usize = 4096;
const EVENT_QUEUE: usize = 16_384;

/// State shared with the MIDI thread.
pub struct Shared {
    pub cmd: Mutex<rtrb::Producer<EngineCommand>>,
    /// Track id that live input goes to (0 = none).
    pub selected: AtomicU64,
    /// Which pitches are currently held (for the on-screen keyboard).
    pub held: [AtomicBool; 128],
}

impl Shared {
    pub fn send(&self, c: EngineCommand) {
        if let Ok(mut tx) = self.cmd.lock() {
            let _ = tx.push(c);
        }
    }

    pub fn note_on(&self, pitch: u8, velocity: u8) {
        self.held[pitch as usize & 127].store(true, Ordering::Relaxed);
        let track = self.selected.load(Ordering::Relaxed);
        if track != 0 {
            self.send(EngineCommand::NoteOn {
                track,
                pitch,
                velocity,
            });
        }
    }

    pub fn note_off(&self, pitch: u8) {
        self.held[pitch as usize & 127].store(false, Ordering::Relaxed);
        let track = self.selected.load(Ordering::Relaxed);
        if track != 0 {
            self.send(EngineCommand::NoteOff { track, pitch });
        }
    }

    /// Release everything held on the selected track (e.g. keyboard unplugged).
    pub fn release_all(&self) {
        for h in &self.held {
            h.store(false, Ordering::Relaxed);
        }
        let track = self.selected.load(Ordering::Relaxed);
        if track != 0 {
            self.send(EngineCommand::AllNotesOff { track });
        }
    }

    pub fn is_held(&self, pitch: u8) -> bool {
        self.held[pitch as usize & 127].load(Ordering::Relaxed)
    }
}

pub type Queue<T> = (rtrb::Producer<T>, rtrb::Consumer<T>);

/// Fresh (command, event) queues for linking the UI to an engine.
pub fn queues() -> (Queue<EngineCommand>, Queue<EngineEvent>) {
    (
        rtrb::RingBuffer::new(COMMAND_QUEUE),
        rtrb::RingBuffer::new(EVENT_QUEUE),
    )
}

pub struct Studio {
    pub project: Project,
    pub shared: Arc<Shared>,
    pub events: rtrb::Consumer<EngineEvent>,
    pub position: f64,
    pub playing: bool,
    pub recording: bool,
    pending: Vec<(u64, Note)>,
    /// The engine is still finishing a take from a project we replaced; ignore it.
    discard_take: bool,
    pub sample_rate: u32,
    /// Sound id each track's engine instrument was built from.
    loaded: HashMap<u64, String>,
    /// Sounds that failed to load: (track name, message), for the UI to show.
    pub load_errors: Vec<String>,
    /// Tracks whose sound failed to load (e.g. a pack not installed yet).
    failed: std::collections::HashSet<u64>,
}

impl Studio {
    pub fn new(
        project: Project,
        cmd: rtrb::Producer<EngineCommand>,
        events: rtrb::Consumer<EngineEvent>,
    ) -> Self {
        let selected = project.tracks.first().map_or(0, |t| t.id);
        let mut s = Self {
            project,
            shared: Arc::new(Shared {
                cmd: Mutex::new(cmd),
                selected: AtomicU64::new(selected),
                held: std::array::from_fn(|_| AtomicBool::new(false)),
            }),
            events,
            position: 0.0,
            playing: false,
            recording: false,
            pending: Vec::new(),
            discard_take: false,
            sample_rate: 48_000,
            loaded: HashMap::new(),
            load_errors: Vec::new(),
            failed: Default::default(),
        };
        s.sync();
        s
    }

    /// A first-launch project: one "Keys" track, ready to play.
    pub fn starter_project() -> Project {
        let mut p = Project::default();
        p.add_instrument_track("Keys");
        p
    }

    /// Push the current project, and any instruments whose sound changed, to the engine.
    pub fn sync(&mut self) {
        // Take heavy instruments back from tracks about to disappear, so the engine
        // never has to free them.
        let gone: Vec<u64> = self
            .loaded
            .keys()
            .filter(|id| self.project.track(**id).and_then(|t| t.sound()).is_none())
            .copied()
            .collect();
        for track in gone {
            self.loaded.remove(&track);
            self.shared.send(EngineCommand::SetInstrument {
                track,
                sound: 0,
                instrument: Box::new(Instrument::None),
            });
        }
        self.shared
            .send(EngineCommand::SetProject(Arc::new(self.project.clone())));
        let changed: Vec<(u64, String)> = self
            .project
            .tracks
            .iter()
            .filter_map(|t| Some((t.id, t.sound()?.to_string())))
            .filter(|(id, s)| self.loaded.get(id) != Some(s))
            .collect();
        for (track, sound) in changed {
            let instrument = match crate::sounds::load(&sound, self.sample_rate) {
                Ok(i) => {
                    self.failed.remove(&track);
                    i
                }
                Err(e) => {
                    self.failed.insert(track);
                    self.load_errors
                        .push(format!("Could not load \"{sound}\": {e}"));
                    Instrument::None
                }
            };
            self.shared.send(EngineCommand::SetInstrument {
                track,
                sound: sound_hash(&sound),
                instrument: Box::new(instrument),
            });
            self.loaded.insert(track, sound);
        }
    }

    /// Try again to load sounds that failed before (after installing a pack).
    pub fn reload_missing(&mut self) {
        for t in std::mem::take(&mut self.failed) {
            self.loaded.remove(&t);
        }
        self.sync();
    }

    /// The engine now runs at `sr`: rebuild every instrument for it.
    pub fn set_sample_rate(&mut self, sr: u32) {
        self.sample_rate = sr;
        self.loaded.clear();
        self.sync();
    }

    pub fn selected(&self) -> u64 {
        self.shared.selected.load(Ordering::Relaxed)
    }

    pub fn select(&self, track: u64) {
        let old = self.selected();
        if old != track {
            self.shared.release_all();
            self.shared.selected.store(track, Ordering::Relaxed);
        }
    }

    /// Swap in a different project (new/open).
    pub fn replace_project(&mut self, project: Project) {
        self.discard_take = self.recording;
        self.shared.send(EngineCommand::Stop);
        self.shared.release_all();
        self.project = project;
        self.pending.clear();
        self.playing = false;
        self.recording = false;
        self.sync();
        self.shared.send(EngineCommand::Locate(0.0));
        let first = self.project.tracks.first().map_or(0, |t| t.id);
        self.shared.selected.store(first, Ordering::Relaxed);
    }

    /// Reconnect to a new engine (audio device restarted).
    pub fn reconnect(
        &mut self,
        cmd: rtrb::Producer<EngineCommand>,
        events: rtrb::Consumer<EngineEvent>,
    ) {
        if let Ok(mut tx) = self.shared.cmd.lock() {
            *tx = cmd;
        }
        self.events = events;
        self.playing = false;
        self.recording = false;
        self.pending.clear();
        self.loaded.clear();
        self.sync();
    }

    pub fn drain_events(&mut self) {
        while let Ok(e) = self.events.pop() {
            self.apply_event(e);
        }
    }

    pub fn apply_event(&mut self, e: EngineEvent) {
        match e {
            EngineEvent::Position(b) => self.position = b,
            EngineEvent::RecordedNote { .. } if self.discard_take => {}
            EngineEvent::RecordFinished { .. } if self.discard_take => self.discard_take = false,
            EngineEvent::RecordedNote { track, note } => self.pending.push((track, note)),
            EngineEvent::RecordFinished {
                track,
                start_beat,
                length_beats,
            } => {
                let notes: Vec<Note> = self
                    .pending
                    .iter()
                    .filter(|(t, _)| *t == track)
                    .map(|(_, n)| Note {
                        start_beat: n.start_beat - start_beat,
                        ..*n
                    })
                    .collect();
                self.pending.retain(|(t, _)| *t != track);
                if notes.is_empty() {
                    return;
                }
                let id = self.project.new_id();
                if let Some(t) = self.project.track_mut(track) {
                    t.clips.push(Clip {
                        id,
                        start_beat,
                        length_beats,
                        notes,
                    });
                    self.sync();
                }
            }
            EngineEvent::Retired(old) => drop(old),
            EngineEvent::RetiredInstrument(old) => drop(old),
        }
    }

    pub fn play_stop(&mut self) {
        if self.playing {
            self.shared.send(EngineCommand::Stop);
            self.playing = false;
            self.recording = false;
        } else {
            self.shared.send(EngineCommand::Play);
            self.playing = true;
        }
    }

    pub fn record(&mut self) {
        if self.recording {
            self.play_stop();
            return;
        }
        let track = self.selected();
        if track == 0 {
            return;
        }
        self.shared.send(EngineCommand::Record { track });
        self.recording = true;
        self.playing = true;
    }

    pub fn rewind(&mut self) {
        self.shared.send(EngineCommand::Locate(0.0));
        self.position = 0.0;
    }

    pub fn locate(&mut self, beat: f64) {
        if self.recording {
            return;
        }
        self.shared.send(EngineCommand::Locate(beat.max(0.0)));
        self.position = beat.max(0.0);
    }
}

/// 1-based (bar, beat in bar) for the display, 4/4: beat 0 → (1, 1), beat 5 → (2, 2).
pub fn bar_and_beat(beat: f64) -> (u64, u64) {
    let beat = beat.max(0.0);
    (
        (beat / 4.0).floor() as u64 + 1,
        (beat % 4.0).floor() as u64 + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn studio() -> (
        Studio,
        rtrb::Consumer<EngineCommand>,
        rtrb::Producer<EngineEvent>,
    ) {
        let ((ctx, crx), (etx, erx)) = queues();
        (Studio::new(Studio::starter_project(), ctx, erx), crx, etx)
    }

    #[test]
    fn splits_bar_and_beat() {
        assert_eq!(bar_and_beat(0.0), (1, 1));
        assert_eq!(bar_and_beat(5.0), (2, 2));
        assert_eq!(bar_and_beat(3.99), (1, 4));
        assert_eq!(bar_and_beat(400.0), (101, 1));
        assert_eq!(bar_and_beat(-2.0), (1, 1));
    }

    #[test]
    fn starter_has_selected_keys_track() {
        let (s, _, _) = studio();
        assert_eq!(s.project.tracks.len(), 1);
        assert_eq!(s.selected(), s.project.tracks[0].id);
    }

    #[test]
    fn apply_event_adds_recorded_clip() {
        let (mut s, _, _) = studio();
        let t = s.selected();
        s.apply_event(EngineEvent::RecordedNote {
            track: t,
            note: Note {
                pitch: 60,
                velocity: 100,
                start_beat: 5.0,
                length_beats: 1.0,
            },
        });
        s.apply_event(EngineEvent::RecordFinished {
            track: t,
            start_beat: 4.0,
            length_beats: 4.0,
        });
        let clips = &s.project.track(t).unwrap().clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].start_beat, 4.0);
        assert_eq!(clips[0].notes[0].start_beat, 1.0);
    }

    #[test]
    fn empty_take_adds_no_clip() {
        let (mut s, _, _) = studio();
        let t = s.selected();
        s.apply_event(EngineEvent::RecordFinished {
            track: t,
            start_beat: 0.0,
            length_beats: 4.0,
        });
        assert!(s.project.track(t).unwrap().clips.is_empty());
    }

    #[test]
    fn notes_go_to_selected_track() {
        let (s, mut crx, _) = studio();
        while crx.pop().is_ok() {}
        s.shared.note_on(60, 100);
        assert!(s.shared.is_held(60));
        match crx.pop() {
            Ok(EngineCommand::NoteOn { track, pitch, .. }) => {
                assert_eq!((track, pitch), (s.selected(), 60))
            }
            _ => panic!("expected NoteOn"),
        }
    }

    #[test]
    fn replacing_project_mid_take_drops_old_take() {
        let (mut s, _, _) = studio();
        s.record();
        s.replace_project(Studio::starter_project());
        let t = s.selected();
        s.apply_event(EngineEvent::RecordedNote {
            track: t,
            note: Note {
                pitch: 60,
                velocity: 100,
                start_beat: 0.0,
                length_beats: 1.0,
            },
        });
        s.apply_event(EngineEvent::RecordFinished {
            track: t,
            start_beat: 0.0,
            length_beats: 4.0,
        });
        assert!(s.project.track(t).unwrap().clips.is_empty());
    }

    #[test]
    fn locate_ignored_while_recording() {
        let (mut s, _, _) = studio();
        s.position = 3.0;
        s.record();
        s.locate(0.0);
        assert_eq!(s.position, 3.0);
    }

    #[test]
    fn unplug_releases_held_notes() {
        let (s, mut crx, _) = studio();
        s.shared.note_on(60, 100);
        while crx.pop().is_ok() {}
        s.shared.release_all();
        assert!(!s.shared.is_held(60));
        assert!(matches!(crx.pop(), Ok(EngineCommand::AllNotesOff { .. })));
    }
}
