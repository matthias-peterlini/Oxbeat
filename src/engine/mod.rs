//! The real-time audio engine. `Engine::process` runs on the audio thread: it never
//! allocates, locks or does I/O.

pub mod commands;
pub mod synth;
pub mod transport;

use std::sync::Arc;

use commands::{EngineCommand, EngineEvent};
use synth::{PRESETS, Synth};
use transport::{beat_to_sample, sample_to_beat};

use crate::project::{Note, Project, TrackKind};

/// Internal processing chunk; bounds scratch buffer size.
const CHUNK: usize = 512;
const MAX_TRACKS: usize = 256;
const BEATS_PER_BAR: f64 = 4.0;
/// Shortest recorded note (a tap within one audio callback).
const MIN_NOTE_BEATS: f64 = 1.0 / 128.0;

struct TrackState {
    id: u64,
    preset: usize,
    synth: Synth,
}

fn preset_index(name: &str) -> usize {
    PRESETS.iter().position(|p| *p == name).unwrap_or(0)
}

struct Recording {
    track: u64,
    start_beat: f64,
    /// Held notes: (start beat, velocity).
    held: [Option<(f64, u8)>; 128],
}

pub struct Engine {
    sample_rate: u32,
    cmd_rx: rtrb::Consumer<EngineCommand>,
    evt_tx: rtrb::Producer<EngineEvent>,
    project: Arc<Project>,
    tracks: Vec<TrackState>,
    playing: bool,
    pos: u64,
    recording: Option<Recording>,
    scratch: [f32; CHUNK],
    since_position: u32,
}

impl Engine {
    pub fn new(
        sample_rate: u32,
        cmd_rx: rtrb::Consumer<EngineCommand>,
        evt_tx: rtrb::Producer<EngineEvent>,
    ) -> Engine {
        Engine {
            sample_rate,
            cmd_rx,
            evt_tx,
            project: Arc::new(Project::default()),
            tracks: Vec::with_capacity(MAX_TRACKS),
            playing: false,
            pos: 0,
            recording: None,
            scratch: [0.0; CHUNK],
            since_position: 0,
        }
    }

    fn beat(&self) -> f64 {
        sample_to_beat(self.pos, self.project.tempo_bpm, self.sample_rate)
    }

    fn send(&mut self, e: EngineEvent) {
        // If the UI isn't draining events the queue may fill; dropping is the only
        // real-time-safe option.
        let _ = self.evt_tx.push(e);
    }

    fn synth_for(&mut self, track: u64) -> Option<&mut Synth> {
        self.tracks
            .iter_mut()
            .find(|t| t.id == track)
            .map(|t| &mut t.synth)
    }

    fn set_project(&mut self, p: Arc<Project>) {
        self.tracks.retain(|t| p.track(t.id).is_some());
        for track in &p.tracks {
            let TrackKind::Instrument { preset } = &track.kind else {
                continue;
            };
            let preset = preset_index(preset);
            let sr = self.sample_rate;
            if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track.id) {
                if t.preset != preset {
                    t.preset = preset;
                    t.synth = Synth::new(PRESETS[preset], sr);
                }
            } else if self.tracks.len() < MAX_TRACKS {
                self.tracks.push(TrackState {
                    id: track.id,
                    preset,
                    synth: Synth::new(PRESETS[preset], sr),
                });
            }
        }
        if p.tempo_bpm != self.project.tempo_bpm && p.tempo_bpm > 0.0 {
            // Keep the playhead on the same beat; scheduled note-offs moved, so release
            // everything that is sounding rather than leave notes stuck.
            let beat = self.beat();
            self.pos = beat_to_sample(beat, p.tempo_bpm, self.sample_rate);
            self.all_off();
        }
        let old = std::mem::replace(&mut self.project, p);
        self.send(EngineEvent::Retired(old));
    }

    fn all_off(&mut self) {
        for t in &mut self.tracks {
            t.synth.all_notes_off();
        }
    }

    fn stop(&mut self) {
        let beat = self.beat();
        if let Some(rec) = self.recording.take() {
            for (pitch, held) in rec.held.iter().enumerate() {
                if let Some((start, velocity)) = held {
                    self.send(EngineEvent::RecordedNote {
                        track: rec.track,
                        note: Note {
                            pitch: pitch as u8,
                            velocity: *velocity,
                            start_beat: *start,
                            length_beats: (beat - start).max(MIN_NOTE_BEATS),
                        },
                    });
                }
            }
            let bars = ((beat - rec.start_beat) / BEATS_PER_BAR).ceil().max(1.0);
            self.send(EngineEvent::RecordFinished {
                track: rec.track,
                start_beat: rec.start_beat,
                length_beats: bars * BEATS_PER_BAR,
            });
        }
        self.playing = false;
        self.all_off();
        let b = self.beat();
        self.send(EngineEvent::Position(b));
    }

    fn handle(&mut self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::SetProject(p) => self.set_project(p),
            EngineCommand::NoteOn {
                track,
                pitch,
                velocity,
            } => {
                let beat = self.beat();
                if let Some(rec) = &mut self.recording
                    && rec.track == track
                    && let Some(slot) = rec.held.get_mut(pitch as usize)
                {
                    *slot = Some((beat, velocity));
                }
                if let Some(s) = self.synth_for(track) {
                    s.note_on(pitch, velocity);
                }
            }
            EngineCommand::NoteOff { track, pitch } => {
                let beat = self.beat();
                let mut finished = None;
                if let Some(rec) = &mut self.recording
                    && rec.track == track
                    && let Some(Some((start, velocity))) =
                        rec.held.get_mut(pitch as usize).map(Option::take)
                {
                    finished = Some(Note {
                        pitch,
                        velocity,
                        start_beat: start,
                        length_beats: (beat - start).max(MIN_NOTE_BEATS),
                    });
                }
                if let Some(note) = finished {
                    self.send(EngineEvent::RecordedNote { track, note });
                }
                if let Some(s) = self.synth_for(track) {
                    s.note_off(pitch);
                }
            }
            EngineCommand::AllNotesOff { track } => {
                let beat = self.beat();
                let mut done = [None; 128];
                if let Some(rec) = &mut self.recording
                    && rec.track == track
                {
                    for (i, h) in rec.held.iter_mut().enumerate() {
                        done[i] = h.take();
                    }
                }
                for (pitch, held) in done.iter().enumerate() {
                    if let Some((start, velocity)) = held {
                        self.send(EngineEvent::RecordedNote {
                            track,
                            note: Note {
                                pitch: pitch as u8,
                                velocity: *velocity,
                                start_beat: *start,
                                length_beats: (beat - start).max(MIN_NOTE_BEATS),
                            },
                        });
                    }
                }
                if let Some(s) = self.synth_for(track) {
                    s.all_notes_off();
                }
            }
            EngineCommand::Play => self.playing = true,
            EngineCommand::Stop => self.stop(),
            EngineCommand::Record { track } => {
                if self.recording.is_some() {
                    return;
                }
                let bar_start = (self.beat() / BEATS_PER_BAR).floor() * BEATS_PER_BAR;
                self.recording = Some(Recording {
                    track,
                    start_beat: bar_start,
                    held: [None; 128],
                });
                self.playing = true;
            }
            EngineCommand::Locate(beat) => {
                // Jumping mid-take would corrupt the recording.
                if self.recording.is_some() {
                    return;
                }
                self.all_off();
                self.pos = beat_to_sample(beat.max(0.0), self.project.tempo_bpm, self.sample_rate);
                let b = self.beat();
                self.send(EngineEvent::Position(b));
            }
        }
    }

    /// Fill `out` (interleaved, `channels` per frame) with the next block of audio.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(cmd) = self.cmd_rx.pop() {
            self.handle(cmd);
        }
        out.fill(0.0);
        let channels = channels.max(1);
        let frames = out.len() / channels;
        let mut done = 0;
        while done < frames {
            let n = CHUNK.min(frames - done);
            self.render_chunk(
                &mut out[done * channels..(done + n) * channels],
                n,
                channels,
            );
            done += n;
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    fn render_chunk(&mut self, out: &mut [f32], n: usize, channels: usize) {
        let project = Arc::clone(&self.project);
        let any_solo = project.tracks.iter().any(|t| t.solo);
        let (start, end) = (self.pos, self.pos + n as u64);
        let tempo = project.tempo_bpm;
        let sr = self.sample_rate;

        for track in &project.tracks {
            let Some(state) = self.tracks.iter_mut().find(|t| t.id == track.id) else {
                continue;
            };
            let scratch = &mut self.scratch[..n];
            scratch.fill(0.0);

            // Render in segments split at note events so timing is sample-accurate.
            let mut cur = start;
            loop {
                if self.playing {
                    // Fire every event at exactly `cur`; find the next one after it.
                    // Note-offs first, so a note ending where a same-pitch note starts
                    // doesn't cut the new one.
                    let mut next = end;
                    for pass_on in [false, true] {
                        for clip in &track.clips {
                            for note in &clip.notes {
                                let on_beat = clip.start_beat + note.start_beat;
                                let on = beat_to_sample(on_beat, tempo, sr);
                                // A note always lasts at least one sample, or it would never
                                // be switched off.
                                let off = beat_to_sample(on_beat + note.length_beats, tempo, sr)
                                    .max(on + 1);
                                if pass_on {
                                    if on == cur {
                                        state.synth.note_on(note.pitch, note.velocity);
                                    }
                                    for t in [on, off] {
                                        if t > cur && t < next {
                                            next = t;
                                        }
                                    }
                                } else if off == cur {
                                    state.synth.note_off(note.pitch);
                                }
                            }
                        }
                    }
                    let a = (cur - start) as usize;
                    let b = (next - start) as usize;
                    state.synth.render(&mut scratch[a..b]);
                    if next >= end {
                        break;
                    }
                    cur = next;
                } else {
                    state.synth.render(scratch);
                    break;
                }
            }

            let audible = !track.mute && (!any_solo || track.solo);
            if !audible {
                continue;
            }
            let angle = (track.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let (l, r) = (angle.cos() * track.volume, angle.sin() * track.volume);
            for (i, s) in scratch.iter().enumerate() {
                let frame = &mut out[i * channels..(i + 1) * channels];
                if channels == 1 {
                    frame[0] += s * (l + r) * 0.5;
                } else {
                    frame[0] += s * l;
                    frame[1] += s * r;
                }
            }
        }

        if self.playing {
            self.pos = end;
            self.since_position += n as u32;
            if self.since_position >= sr / 30 {
                self.since_position = 0;
                let b = self.beat();
                self.send(EngineEvent::Position(b));
            }
        }
        // Return our extra reference without freeing on this thread
        // (the UI or the next SetProject still holds the Arc).
        drop(project);
    }
}

#[cfg(test)]
mod tests;
