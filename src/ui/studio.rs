//! Non-visual app state: the project, the engine link, recording assembly.
//! Kept free of egui so it can be unit-tested.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::history::History;
use super::loader::{Loaded, Loader};
use crate::audio::recorder::Recorder;
use crate::engine::commands::{EngineCommand, EngineEvent, METER_TRACKS, Meters, UiEnd};
use crate::engine::fx::Chain;
use crate::engine::instrument::{Instrument, sound_hash};
use crate::engine::sampler::SampleData;
use crate::project::effects::EffectKind;
use crate::project::{Clip, Note, Project, TrackKind};

/// Voice takes shorter than this (an accidental tap of R) are thrown away.
const MIN_TAKE_SECONDS: f64 = 0.05;
/// How much of a meter's level is left one screen frame later.
const METER_FALL: f32 = 0.85;
/// Frames summarised by each waveform peak.
const FRAMES_PER_PEAK: usize = 256;

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

/// A loudness outline of an audio clip, for drawing.
pub struct Waveform {
    /// Loudest sample in each run of `FRAMES_PER_PEAK` frames, 0.0..=1.0.
    pub peaks: Vec<f32>,
    pub seconds_per_peak: f64,
    pub seconds: f64,
}

impl Waveform {
    pub fn of(data: &SampleData) -> Self {
        let peaks = (0..data.len())
            .step_by(FRAMES_PER_PEAK)
            .map(|start| {
                let end = (start + FRAMES_PER_PEAK).min(data.len());
                (start..end)
                    .map(|i| {
                        let (l, r) = data.frame(i);
                        l.abs().max(r.abs())
                    })
                    .fold(0.0f32, f32::max)
                    .min(1.0)
            })
            .collect();
        let rate = data.rate.max(1) as f64;
        Self {
            peaks,
            seconds_per_peak: FRAMES_PER_PEAK as f64 / rate,
            seconds: data.len() as f64 / rate,
        }
    }

    /// Loudest peak between two times (seconds into the clip).
    pub fn peak_between(&self, from: f64, to: f64) -> f32 {
        let a = (from / self.seconds_per_peak).floor().max(0.0) as usize;
        let b = ((to / self.seconds_per_peak).ceil() as usize).max(a + 1);
        self.peaks
            .get(a..b.min(self.peaks.len()))
            .map_or(0.0, |s| s.iter().fold(0.0f32, |m, x| m.max(*x)))
    }
}

/// A fresh folder for the takes of a project that hasn't been saved yet.
pub fn unsaved_audio_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    crate::sounds::library::data_dir()
        .join("unsaved")
        .join(stamp.to_string())
}

/// An audio clip the engine has: its file and how long it plays.
struct LoadedAudio {
    file: String,
    seconds: f64,
}

pub struct Studio {
    pub project: Project,
    history: History,
    /// The project was opened from, or saved to, disk.
    on_disk: bool,
    /// Essentials arrived mid-playback or mid-take; upgrade the starter once stopped.
    starter_upgrade_waiting: bool,
    /// The clip picked in the arrangement, if any.
    pub selected_clip: Option<u64>,
    /// Click on every beat while playing.
    pub metronome: bool,
    /// Count one bar in before recording.
    pub count_in: bool,
    pub loop_on: bool,
    /// Beats the loop runs between.
    pub loop_region: (f64, f64),
    pub shared: Arc<Shared>,
    pub events: rtrb::Consumer<EngineEvent>,
    meters: Arc<Meters>,
    recorder: Recorder,
    pub position: f64,
    pub playing: bool,
    pub recording: bool,
    /// The take in progress: (track, beat it started on).
    pub recording_from: Option<(u64, f64)>,
    pending: Vec<(u64, Note)>,
    /// The voice take being written: (track, clip id it will become).
    voice_take: Option<(u64, u64)>,
    /// The engine is still finishing a take from a project we replaced; ignore it.
    discard_take: bool,
    pub sample_rate: u32,
    /// Where this project's recordings live (`<bundle>/audio` once saved).
    pub audio_dir: PathBuf,
    /// Set by the app while a microphone is open.
    pub input_name: Option<String>,
    /// Microphone level for the meter, 0.0..=1.0, falling smoothly.
    pub input_level: f32,
    /// Level of each track (by position in the project) and of the final mix.
    pub track_levels: Vec<f32>,
    pub master_level: [f32; 2],
    /// Kinds of effect in the chain the engine has for each track.
    fx_sent: HashMap<u64, Vec<EffectKind>>,
    /// Sound id each track's engine instrument was built (or is being built) from.
    loaded: HashMap<u64, String>,
    /// Tracks waiting for a file or pack instrument from the loader -> its sound id.
    /// A newer request for a track replaces the older one.
    loading: HashMap<u64, String>,
    loader: Loader,
    /// Audio clips the engine has been given, by clip id.
    loaded_audio: HashMap<u64, LoadedAudio>,
    pub waveforms: HashMap<u64, Arc<Waveform>>,
    /// Sounds that failed to load: (track name, message), for the UI to show.
    pub load_errors: Vec<String>,
    /// Tracks whose sound failed to load (e.g. a pack not installed yet).
    failed: HashSet<u64>,
}

impl Studio {
    #[cfg(test)]
    pub fn new(project: Project, link: UiEnd) -> Self {
        Self::with_sample_rate(project, link, 48_000)
    }

    /// For an engine already running at `sample_rate`, so instruments are built
    /// once, at the right rate.
    pub fn with_sample_rate(project: Project, link: UiEnd, sample_rate: u32) -> Self {
        let selected = project.tracks.first().map_or(0, |t| t.id);
        let mut s = Self {
            history: History::new(&project),
            on_disk: false,
            starter_upgrade_waiting: false,
            selected_clip: None,
            metronome: false,
            count_in: false,
            loop_on: false,
            loop_region: (0.0, 16.0),
            project,
            shared: Arc::new(Shared {
                cmd: Mutex::new(link.commands),
                selected: AtomicU64::new(selected),
                held: std::array::from_fn(|_| AtomicBool::new(false)),
            }),
            events: link.events,
            meters: link.meters,
            recorder: Recorder::new(link.recorded),
            position: 0.0,
            playing: false,
            recording: false,
            recording_from: None,
            pending: Vec::new(),
            voice_take: None,
            discard_take: false,
            sample_rate,
            audio_dir: unsaved_audio_dir(),
            input_name: None,
            input_level: 0.0,
            track_levels: Vec::new(),
            master_level: [0.0; 2],
            fx_sent: HashMap::new(),
            loaded: HashMap::new(),
            loading: HashMap::new(),
            loader: Loader::sounds(),
            loaded_audio: HashMap::new(),
            waveforms: HashMap::new(),
            load_errors: Vec::new(),
            failed: HashSet::new(),
        };
        s.sync();
        s
    }

    /// A first-launch project: one piano track, ready to play.
    pub fn starter_project(essentials_installed: bool) -> Project {
        let mut p = Project::default();
        p.add_instrument_track(crate::sounds::default_instrument(essentials_installed));
        p
    }

    /// The project now has a file on disk (opened or saved): no longer a blank starter.
    pub fn mark_on_disk(&mut self) {
        self.on_disk = true;
    }

    /// Essentials just arrived: move an untouched starter project from "Keys" onto the
    /// grand piano. Returns true when it did (the caller then syncs). While playing or
    /// recording it waits, and `drain_events` applies it once stopped.
    pub fn upgrade_starter_track(&mut self) -> bool {
        if self.playing || self.recording {
            self.starter_upgrade_waiting = true;
            return false;
        }
        self.starter_upgrade_waiting = false;
        let untouched = !self.on_disk
            && !self.history.can_undo()
            && self.project.tracks.len() == 1
            && self.project.tracks[0].clips.is_empty()
            && matches!(
                &self.project.tracks[0].kind,
                TrackKind::Instrument { preset } if preset == "Keys"
            );
        if !untouched {
            return false;
        }
        let t = &mut self.project.tracks[0];
        t.kind = TrackKind::Instrument {
            preset: crate::sounds::ESSENTIALS_PIANO.to_string(),
        };
        t.name = "Grand Piano".to_string();
        self.history.reset(&self.project);
        true
    }

    /// Push the current project, and any instruments or audio that changed, to the engine.
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
            self.loading.remove(&track);
            self.shared.send(EngineCommand::SetInstrument {
                track,
                sound: 0,
                instrument: Box::new(Instrument::None),
            });
        }
        self.sync_audio();
        self.shared
            .send(EngineCommand::SetProject(Arc::new(self.project.clone())));
        self.shared.send(EngineCommand::SetArmed(self.selected()));
        self.sync_effects();
        let changed: Vec<(u64, String)> = self
            .project
            .tracks
            .iter()
            .filter_map(|t| Some((t.id, t.sound()?.to_string())))
            .filter(|(id, s)| self.loaded.get(id) != Some(s))
            .collect();
        for (track, sound) in changed {
            // Built-in synths and kits are instant; anything else is decoded on the
            // loader's thread and handed over in `poll_loads`. Until then the track
            // is silent or keeps its previous instrument.
            if let Some(instrument) = Instrument::builtin(&sound, self.sample_rate) {
                self.failed.remove(&track);
                self.loading.remove(&track);
                self.shared.send(EngineCommand::SetInstrument {
                    track,
                    sound: sound_hash(&sound),
                    instrument: Box::new(instrument),
                });
            } else {
                self.loader.request(&sound, self.sample_rate);
                self.loading.insert(track, sound.clone());
            }
            self.loaded.insert(track, sound);
        }
        if self
            .selected_clip
            .is_some_and(|c| self.project.clip(c).is_none())
        {
            self.selected_clip = None;
        }
        self.history.record(&self.project, Instant::now());
    }

    /// Wake the UI when a background load finishes.
    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        self.loader.set_repaint(f);
    }

    #[cfg(test)]
    fn set_loader(&mut self, loader: Loader) {
        self.loader = loader;
    }

    /// Hand finished background loads to the engine. Call every frame.
    pub fn poll_loads(&mut self) {
        while let Some(l) = self.loader.try_recv() {
            self.apply_loaded(l);
        }
    }

    fn apply_loaded(&mut self, l: Loaded) {
        // Built for an engine rate we no longer run at: a fresh request is queued.
        if l.sample_rate != self.sample_rate {
            return;
        }
        let mut tracks: Vec<u64> = self
            .loading
            .iter()
            .filter(|(_, s)| **s == l.sound)
            .map(|(t, _)| *t)
            .collect();
        // No track wants this sound any more (changed or deleted): drop it.
        if tracks.is_empty() {
            return;
        }
        tracks.sort_unstable();
        let hash = sound_hash(&l.sound);
        match l.result {
            Ok(instrument) => {
                let track = tracks[0];
                self.loading.remove(&track);
                self.failed.remove(&track);
                self.shared.send(EngineCommand::SetInstrument {
                    track,
                    sound: hash,
                    instrument: Box::new(instrument),
                });
                // Every track needs its own instrument; the samples are cached now.
                if tracks.len() > 1 {
                    self.loader.request(&l.sound, self.sample_rate);
                }
            }
            Err(e) => {
                self.load_errors
                    .push(format!("Could not load \"{}\": {e}", l.sound));
                for track in tracks {
                    self.loading.remove(&track);
                    self.failed.insert(track);
                    self.shared.send(EngineCommand::SetInstrument {
                        track,
                        sound: hash,
                        instrument: Box::new(Instrument::None),
                    });
                }
            }
        }
    }

    pub fn set_metronome(&mut self, on: bool) {
        self.metronome = on;
        self.shared.send(EngineCommand::SetMetronome(on));
    }

    /// Switch looping on or off. Switching it on with a clip picked loops that clip.
    pub fn set_loop(&mut self, on: bool) {
        if on && let Some((_, clip)) = self.selected_clip.and_then(|c| self.project.clip(c)) {
            let from = clip.start_beat.floor();
            self.loop_region = (from, clip.end_beat().ceil().max(from + 1.0));
        }
        self.loop_on = on;
        self.send_loop();
    }

    /// Move the loop; it always covers at least one beat.
    pub fn set_loop_region(&mut self, from: f64, to: f64) {
        let from = from.max(0.0);
        self.loop_region = (from, to.max(from + 1.0));
        self.send_loop();
    }

    fn send_loop(&self) {
        self.shared.send(EngineCommand::SetLoop(
            self.loop_on.then_some(self.loop_region),
        ));
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo() && !self.recording
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo() && !self.recording
    }

    pub fn undo(&mut self) {
        if self.can_undo()
            && let Some(p) = self.history.undo()
        {
            self.restore(p);
        }
    }

    pub fn redo(&mut self) {
        if self.can_redo()
            && let Some(p) = self.history.redo()
        {
            self.restore(p);
        }
    }

    /// Put back a project from the history.
    fn restore(&mut self, project: Project) {
        self.shared.release_all();
        self.project = project;
        if self.project.track(self.selected()).is_none() {
            let first = self.project.tracks.first().map_or(0, |t| t.id);
            self.shared.selected.store(first, Ordering::Relaxed);
        }
        self.sync();
    }

    /// Pick a clip (and the track it is on).
    pub fn select_clip(&mut self, clip: u64) {
        if let Some((track, _)) = self.project.clip(clip) {
            let track = track.id;
            self.select(track);
            self.selected_clip = Some(clip);
        }
    }

    pub fn delete_selected_clip(&mut self) {
        if let Some(clip) = self.selected_clip.take()
            && self.project.remove_clip(clip)
        {
            self.sync();
        }
    }

    pub fn duplicate_selected_clip(&mut self) {
        if let Some(clip) = self.selected_clip
            && let Some(copy) = self.project.duplicate_clip(clip)
        {
            self.selected_clip = Some(copy);
            self.sync();
        }
    }

    /// Cut the selected clip in two at the playhead.
    pub fn split_selected_clip(&mut self) {
        if let Some(clip) = self.selected_clip
            && self.project.split_clip(clip, self.position).is_some()
        {
            self.sync();
        }
    }

    pub fn delete_track(&mut self, track: u64) {
        if self.recording || !self.project.remove_track(track) {
            return;
        }
        if self.selected() == track {
            self.shared.release_all();
            let first = self.project.tracks.first().map_or(0, |t| t.id);
            self.shared.selected.store(first, Ordering::Relaxed);
        }
        self.sync();
    }

    /// Rebuild the effect chain of every track whose list of effects changed (knob
    /// positions travel with the project itself).
    fn sync_effects(&mut self) {
        // The engine hands back the chains of deleted tracks by itself.
        self.fx_sent
            .retain(|id, _| self.project.track(*id).is_some());
        for t in &self.project.tracks {
            let kinds = Chain::kinds(&t.effects);
            let sent = self.fx_sent.get(&t.id);
            if sent == Some(&kinds) || (sent.is_none() && kinds.is_empty()) {
                continue;
            }
            self.shared.send(EngineCommand::SetEffects {
                track: t.id,
                chain: Box::new(Chain::build(&t.effects, self.sample_rate)),
            });
            self.fx_sent.insert(t.id, kinds);
        }
    }

    /// Give the engine the audio of new clips, take back removed ones, and keep audio
    /// clips as long as their recording at the current tempo.
    fn sync_audio(&mut self) {
        let wanted: HashMap<u64, String> = self
            .project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter_map(|c| Some((c.id, c.audio.clone()?)))
            .collect();
        let stale: Vec<u64> = self
            .loaded_audio
            .iter()
            .filter(|(id, a)| wanted.get(id) != Some(&a.file))
            .map(|(id, _)| *id)
            .collect();
        for clip in stale {
            self.loaded_audio.remove(&clip);
            self.waveforms.remove(&clip);
            self.shared
                .send(EngineCommand::SetAudio { clip, data: None });
        }
        for (clip, file) in wanted {
            if self.loaded_audio.contains_key(&clip) {
                continue;
            }
            let seconds = match crate::sounds::decode::decode_file(&self.audio_dir.join(&file)) {
                Ok(data) => {
                    let wave = Waveform::of(&data);
                    let seconds = wave.seconds;
                    self.waveforms.insert(clip, Arc::new(wave));
                    self.shared.send(EngineCommand::SetAudio {
                        clip,
                        data: Some(Arc::new(data)),
                    });
                    seconds
                }
                Err(e) => {
                    self.load_errors
                        .push(format!("Could not load the recording \"{file}\": {e}"));
                    0.0
                }
            };
            self.loaded_audio
                .insert(clip, LoadedAudio { file, seconds });
        }
        // A new clip plays its whole recording; no clip may run past the end of it.
        let beats_per_second = self.project.tempo_bpm / 60.0;
        for clip in self.project.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if let Some(a) = self.loaded_audio.get(&clip.id)
                && a.seconds > 0.0
            {
                let most = (a.seconds - clip.audio_offset).max(0.0) * beats_per_second;
                if clip.length_beats <= 0.0 || clip.length_beats > most {
                    clip.length_beats = most;
                }
            }
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
        self.fx_sent.clear();
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
            self.shared.send(EngineCommand::SetArmed(track));
        }
    }

    /// True when the selected track records the microphone.
    pub fn voice_selected(&self) -> bool {
        self.project
            .track(self.selected())
            .is_some_and(|t| t.is_audio())
    }

    /// The project has a voice track, so the microphone should be open.
    pub fn needs_input(&self) -> bool {
        self.project.tracks.iter().any(|t| t.is_audio())
    }

    /// Hand the engine a newly opened microphone (or `None` when it closed).
    pub fn set_input(&mut self, input: Option<(String, rtrb::Consumer<f32>)>) {
        let (name, queue) = input.unzip();
        self.input_name = name;
        self.shared.send(EngineCommand::SetInput(queue));
    }

    /// Advance the level meters one frame.
    pub fn update_meter(&mut self) {
        let peak = self.meters.take_input().min(1.0);
        self.input_level = peak.max(self.input_level * METER_FALL);
        let tracks = self.project.tracks.len().min(METER_TRACKS);
        self.track_levels.resize(tracks, 0.0);
        for (i, level) in self.track_levels.iter_mut().enumerate() {
            *level = self.meters.take_track(i).max(*level * METER_FALL);
        }
        for (level, now) in self.master_level.iter_mut().zip(self.meters.take_master()) {
            *level = now.max(*level * METER_FALL);
        }
    }

    /// True while any meter still shows something (so the UI keeps redrawing).
    pub fn meters_moving(&self) -> bool {
        self.master_level.iter().any(|l| *l > 0.001)
    }

    /// Swap in a different project (new/open), whose recordings live in `audio_dir`.
    pub fn replace_project(&mut self, project: Project, audio_dir: PathBuf) {
        self.discard_take = self.recording;
        self.shared.send(EngineCommand::Stop);
        self.shared.release_all();
        if self.voice_take.take().is_some() {
            self.recorder.cancel();
        }
        self.project = project;
        self.history.reset(&self.project);
        self.on_disk = false;
        self.selected_clip = None;
        self.audio_dir = audio_dir;
        self.pending.clear();
        self.playing = false;
        self.recording = false;
        self.recording_from = None;
        let first = self.project.tracks.first().map_or(0, |t| t.id);
        self.shared.selected.store(first, Ordering::Relaxed);
        self.sync();
        self.shared.send(EngineCommand::Locate(0.0));
    }

    /// Reconnect to a new engine (audio device restarted).
    pub fn reconnect(&mut self, link: UiEnd) {
        if let Ok(mut tx) = self.shared.cmd.lock() {
            *tx = link.commands;
        }
        self.events = link.events;
        self.meters = link.meters;
        self.recorder.set_source(link.recorded);
        if self.voice_take.take().is_some() {
            self.recorder.cancel();
        }
        self.playing = false;
        self.recording = false;
        self.recording_from = None;
        self.pending.clear();
        self.loaded.clear();
        self.fx_sent.clear();
        self.loaded_audio.clear();
        self.input_name = None;
        self.sync();
        self.shared
            .send(EngineCommand::SetMetronome(self.metronome));
        self.send_loop();
    }

    /// Apply engine events and finished instrument loads. Call every frame.
    pub fn drain_events(&mut self) {
        while let Ok(e) = self.events.pop() {
            self.apply_event(e);
        }
        self.poll_loads();
        if self.starter_upgrade_waiting && !self.playing && !self.recording {
            // Still decided by whether the project is untouched now.
            if self.upgrade_starter_track() {
                self.sync();
            }
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
                self.recording_from = None;
                if let Some((voice_track, clip)) = self.voice_take
                    && voice_track == track
                {
                    self.voice_take = None;
                    self.finish_voice_take(track, clip, start_beat);
                    return;
                }
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
                        audio: None,
                        audio_offset: 0.0,
                    });
                    self.sync();
                }
            }
            EngineEvent::Retired(old) => drop(old),
            EngineEvent::RetiredInstrument(old) => drop(old),
            EngineEvent::RetiredAudio(old) => drop(old),
            EngineEvent::RetiredEffects(old) => drop(old),
            EngineEvent::RetiredInput(old) => drop(old),
        }
    }

    fn finish_voice_take(&mut self, track: u64, clip: u64, start_beat: f64) {
        let take = match self.recorder.stop() {
            Ok(t) => t,
            Err(e) => {
                self.load_errors.push(e);
                return;
            }
        };
        if (take.frames as f64) < MIN_TAKE_SECONDS * self.sample_rate as f64 {
            let _ = std::fs::remove_file(&take.path);
            return;
        }
        let file = take
            .path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned());
        if let (Some(t), Some(file)) = (self.project.track_mut(track), file) {
            t.clips.push(Clip {
                id: clip,
                start_beat,
                length_beats: 0.0,
                notes: Vec::new(),
                audio: Some(file),
                audio_offset: 0.0,
            });
            self.sync();
        }
    }

    /// Put an audio file on the selected voice track at the playhead. The audio is
    /// copied into the project, so the original can move or disappear.
    pub fn import_audio(&mut self, source: &Path) -> Result<(), String> {
        let track = self.selected();
        if !self.voice_selected() {
            return Err("Select a voice track first.".into());
        }
        let data = crate::sounds::decode::decode_file(source)?;
        let clip = self.project.new_id();
        let file = format!("import-{clip}.wav");
        std::fs::create_dir_all(&self.audio_dir).map_err(|e| e.to_string())?;
        write_wav(&self.audio_dir.join(&file), &data)?;
        let start_beat = self.position;
        if let Some(t) = self.project.track_mut(track) {
            t.clips.push(Clip {
                id: clip,
                start_beat,
                length_beats: 0.0,
                notes: Vec::new(),
                audio: Some(file),
                audio_offset: 0.0,
            });
        }
        self.sync();
        Ok(())
    }

    /// The project was saved into a bundle: keep its recordings in `dir` from now on.
    pub fn move_audio_to(&mut self, dir: &Path) -> Result<(), String> {
        if dir == self.audio_dir {
            return Ok(());
        }
        let files: Vec<String> = self.project.audio_files().map(str::to_string).collect();
        if !files.is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        for f in &files {
            let from = self.audio_dir.join(f);
            if from.is_file() {
                std::fs::copy(&from, dir.join(f)).map_err(|e| e.to_string())?;
            }
        }
        let old = std::mem::replace(&mut self.audio_dir, dir.to_path_buf());
        // Recordings of a never-saved project were only parked in our data folder.
        if old.starts_with(crate::sounds::library::data_dir().join("unsaved")) {
            let _ = std::fs::remove_dir_all(old);
        }
        Ok(())
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
        if self.voice_selected() {
            if self.input_name.is_none() {
                self.load_errors
                    .push("No microphone is connected. Plug one in to record your voice.".into());
                return;
            }
            let clip = self.project.new_id();
            self.recorder.start(
                self.audio_dir.join(format!("take-{clip}.wav")),
                self.sample_rate,
            );
            self.voice_take = Some((track, clip));
            self.recording_from = Some((track, self.position));
        } else {
            self.recording_from = Some((track, (self.position / 4.0).floor() * 4.0));
        }
        self.shared.send(EngineCommand::Record {
            track,
            count_in: self.count_in,
        });
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

/// Save decoded audio as a 32-bit float stereo WAV.
fn write_wav(path: &Path, data: &SampleData) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: data.rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for i in 0..data.len() {
        let (l, r) = data.frame(i);
        w.write_sample(l).map_err(|e| e.to_string())?;
        w.write_sample(r).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
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
        let (ui, end) = crate::engine::commands::link();
        (
            Studio::new(Studio::starter_project(false), ui),
            end.commands,
            end.events,
        )
    }

    #[test]
    fn starter_upgrade_only_when_untouched() {
        let (mut s, _, _) = studio();
        assert!(s.upgrade_starter_track());
        let t = &s.project.tracks[0];
        assert_eq!(t.sound(), Some(crate::sounds::ESSENTIALS_PIANO));
        assert_eq!(t.name, "Grand Piano");

        let (mut s, _, _) = studio();
        let t = s.selected();
        s.project.add_empty_clip(t, 0.0, 4.0);
        s.sync();
        assert!(!s.upgrade_starter_track());

        let (mut s, _, _) = studio();
        s.replace_project(Studio::starter_project(false), unsaved_audio_dir());
        s.mark_on_disk();
        assert!(!s.upgrade_starter_track());

        let (mut s, _, _) = studio();
        let t = s.selected();
        s.project.track_mut(t).unwrap().kind = crate::project::TrackKind::Instrument {
            preset: "Bass".into(),
        };
        assert!(!s.upgrade_starter_track());
    }

    fn set_sound(s: &mut Studio, track: u64, sound: &str) {
        s.project.track_mut(track).unwrap().kind = TrackKind::Instrument {
            preset: sound.into(),
        };
    }

    /// Sound hashes of the SetInstrument commands sent for `track`.
    fn instruments_sent(crx: &mut rtrb::Consumer<EngineCommand>, track: u64) -> Vec<u64> {
        let mut out = Vec::new();
        while let Ok(c) = crx.pop() {
            if let EngineCommand::SetInstrument {
                track: t, sound, ..
            } = c
                && t == track
            {
                out.push(sound);
            }
        }
        out
    }

    #[test]
    fn file_sounds_load_off_the_calling_thread() {
        let (mut s, mut crx, _) = studio();
        let threads = Arc::new(Mutex::new(Vec::new()));
        let seen = threads.clone();
        s.set_loader(Loader::new(move |_, sr| {
            seen.lock().unwrap().push(std::thread::current().id());
            crate::sounds::load("Keys", sr)
        }));
        let t = s.selected();
        let other = s.project.add_instrument_track("Keys");
        s.sync();
        crx_drain(&mut crx);
        set_sound(&mut s, t, "file:/piano.sfz");
        set_sound(&mut s, other, "file:/piano.sfz");
        s.sync();
        assert!(
            instruments_sent(&mut crx, t).is_empty(),
            "loaded during sync"
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let (mut got_t, mut got_other) = (false, false);
        while !(got_t && got_other) && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            s.poll_loads();
            while let Ok(c) = crx.pop() {
                if let EngineCommand::SetInstrument { track, sound, .. } = c {
                    assert_eq!(sound, sound_hash("file:/piano.sfz"));
                    got_t |= track == t;
                    got_other |= track == other;
                }
            }
        }
        assert!(got_t && got_other, "both tracks get their instrument");
        let threads = threads.lock().unwrap();
        assert_eq!(threads.len(), 2, "one load per track, one at a time");
        assert!(threads.iter().all(|id| *id != std::thread::current().id()));
    }

    fn crx_drain(crx: &mut rtrb::Consumer<EngineCommand>) {
        while crx.pop().is_ok() {}
    }

    #[test]
    fn stale_load_results_are_dropped() {
        let (mut s, mut crx, _) = studio();
        // A loader that never finishes on its own, so only hand-fed results arrive.
        let (gate, wait) = std::sync::mpsc::channel::<()>();
        s.set_loader(Loader::new(move |_, _| {
            let _ = wait.recv();
            Err("cancelled".into())
        }));
        let t = s.selected();
        set_sound(&mut s, t, "file:/a.sfz");
        s.sync();
        set_sound(&mut s, t, "file:/b.sfz");
        s.sync();
        crx_drain(&mut crx);
        let keys = || Ok(Instrument::builtin("Keys", 48_000).unwrap());
        s.apply_loaded(Loaded {
            sound: "file:/a.sfz".into(),
            sample_rate: s.sample_rate,
            result: keys(),
        });
        s.apply_loaded(Loaded {
            sound: "file:/b.sfz".into(),
            sample_rate: s.sample_rate + 1,
            result: keys(),
        });
        assert!(
            instruments_sent(&mut crx, t).is_empty(),
            "stale result applied"
        );
        s.apply_loaded(Loaded {
            sound: "file:/b.sfz".into(),
            sample_rate: s.sample_rate,
            result: keys(),
        });
        assert_eq!(
            instruments_sent(&mut crx, t),
            vec![sound_hash("file:/b.sfz")]
        );
        drop(gate);
    }

    #[test]
    fn starter_upgrade_waits_until_stopped() {
        let piano = Some(crate::sounds::ESSENTIALS_PIANO);
        // Playing: deferred, then applied once stopped.
        let (mut s, _, _) = studio();
        s.play_stop();
        assert!(!s.upgrade_starter_track());
        s.drain_events();
        assert_eq!(s.project.tracks[0].sound(), Some("Keys"));
        s.play_stop();
        s.drain_events();
        assert_eq!(s.project.tracks[0].sound(), piano);
        assert!(!s.can_undo());

        // Recording (or counting in): same.
        let (mut s, _, _) = studio();
        s.record();
        assert!(!s.upgrade_starter_track());
        s.drain_events();
        assert_eq!(s.project.tracks[0].sound(), Some("Keys"));
        s.record(); // stops
        s.drain_events();
        assert_eq!(s.project.tracks[0].sound(), piano);

        // A take recorded meanwhile means the project is no longer untouched.
        let (mut s, _, _) = studio();
        let t = s.selected();
        s.record();
        assert!(!s.upgrade_starter_track());
        s.apply_event(EngineEvent::RecordedNote {
            track: t,
            note: Note {
                pitch: 60,
                velocity: 100,
                start_beat: 0.5,
                length_beats: 1.0,
            },
        });
        s.apply_event(EngineEvent::RecordFinished {
            track: t,
            start_beat: 0.0,
            length_beats: 4.0,
        });
        s.record();
        s.drain_events();
        assert_eq!(s.project.tracks[0].sound(), Some("Keys"));
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
        s.replace_project(Studio::starter_project(false), unsaved_audio_dir());
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

#[cfg(test)]
mod voice_tests {
    use super::*;
    use crate::engine::commands::{EngineEnd, link};

    fn voice_studio(dir: &Path) -> (Studio, u64, EngineEnd) {
        let (ui, end) = link();
        let mut s = Studio::new(Project::default(), ui);
        s.audio_dir = dir.join("audio");
        let t = s.project.add_voice_track();
        s.sync();
        s.select(t);
        (s, t, end)
    }

    #[test]
    fn record_without_microphone_explains_instead_of_recording() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _, _end) = voice_studio(dir.path());
        s.record();
        assert!(!s.recording);
        assert!(s.load_errors[0].contains("microphone"));
    }

    #[test]
    fn voice_take_becomes_an_audio_clip_as_long_as_the_recording() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, t, mut end) = voice_studio(dir.path());
        s.input_name = Some("Mic".into());
        s.position = 2.0;
        s.record();
        assert!(s.recording);
        assert_eq!(s.recording_from, Some((t, 2.0)));
        std::thread::sleep(std::time::Duration::from_millis(60));
        // One second of microphone at 48 kHz = 2 beats at 120 bpm.
        for i in 0..48_000 {
            end.recorded.push((i as f32 * 0.05).sin() * 0.5).unwrap();
        }
        s.apply_event(EngineEvent::RecordFinished {
            track: t,
            start_beat: 2.0,
            length_beats: 2.0,
        });
        assert!(s.load_errors.is_empty(), "{:?}", s.load_errors);
        let clip = &s.project.track(t).unwrap().clips[0];
        assert_eq!(clip.start_beat, 2.0);
        assert!((clip.length_beats - 2.0).abs() < 1e-6);
        let file = clip.audio.clone().unwrap();
        assert!(dir.path().join("audio").join(&file).is_file());
        let wave = &s.waveforms[&clip.id];
        assert!((wave.seconds - 1.0).abs() < 1e-6);
        assert!(wave.peak_between(0.0, 1.0) > 0.4);
        assert_eq!(s.recording_from, None);
    }

    #[test]
    fn accidental_tap_leaves_no_clip_or_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, t, mut end) = voice_studio(dir.path());
        s.input_name = Some("Mic".into());
        s.record();
        std::thread::sleep(std::time::Duration::from_millis(60));
        for _ in 0..100 {
            end.recorded.push(0.1).unwrap();
        }
        s.apply_event(EngineEvent::RecordFinished {
            track: t,
            start_beat: 0.0,
            length_beats: 0.01,
        });
        assert!(s.project.track(t).unwrap().clips.is_empty());
        let left: Vec<_> = std::fs::read_dir(dir.path().join("audio"))
            .unwrap()
            .collect();
        assert!(left.is_empty());
    }

    #[test]
    fn audio_clips_follow_the_tempo_and_saving_copies_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, t, _end) = voice_studio(dir.path());
        let src = dir.path().join("loop.wav");
        write_wav(&src, &SampleData::stereo(44_100, vec![0.25; 44_100 * 2])).unwrap();
        s.position = 4.0;
        s.import_audio(&src).unwrap();
        let clip = s.project.track(t).unwrap().clips[0].clone();
        assert_eq!(clip.start_beat, 4.0);
        assert!((clip.length_beats - 2.0).abs() < 1e-3);

        s.project.tempo_bpm = 60.0;
        s.sync();
        assert!((s.project.track(t).unwrap().clips[0].length_beats - 1.0).abs() < 1e-3);

        let bundle = dir.path().join("Song.oxbeat").join("audio");
        s.move_audio_to(&bundle).unwrap();
        assert!(bundle.join(clip.audio.unwrap()).is_file());
        assert_eq!(s.audio_dir, bundle);
    }

    #[test]
    fn import_needs_a_voice_track() {
        let (ui, _end) = link();
        let mut s = Studio::new(Studio::starter_project(false), ui);
        assert!(s.import_audio(Path::new("nope.wav")).is_err());
    }
}
