//! The real-time audio engine. `Engine::process` runs on the audio thread: it never
//! allocates, locks or does I/O.

pub mod commands;
pub mod drums;
pub mod fx;
pub mod instrument;
pub mod metronome;
pub mod sampler;
pub mod synth;
pub mod transport;

use std::sync::Arc;

use commands::{EngineCommand, EngineEnd, EngineEvent, Meters};
use fx::Chain;
use fx::dynamics::Limiter;
use instrument::{Instrument, sound_hash};
use sampler::SampleData;
use transport::{beat_to_sample, sample_to_beat, samples_per_beat};

use crate::project::{Note, Project, Track};

/// Internal processing chunk; bounds scratch buffer size.
const CHUNK: usize = 512;
const MAX_TRACKS: usize = 256;
/// Audio clips the engine can hold at once.
const MAX_AUDIO_CLIPS: usize = 4096;
const BEATS_PER_BAR: f64 = 4.0;
/// Shortest recorded note (a tap within one audio callback).
const MIN_NOTE_BEATS: f64 = 1.0 / 128.0;
/// Microphone samples allowed to pile up before we skip ahead (input and output
/// clocks drift apart slowly); about 20 ms at 48 kHz.
const MAX_INPUT_BACKLOG: usize = 1024;

struct TrackState {
    id: u64,
    /// `sound_hash` of the sound the instrument was built for.
    sound: u64,
    instrument: Instrument,
}

struct Recording {
    track: u64,
    start_beat: f64,
    /// Recording the microphone (a voice track) rather than notes.
    audio: bool,
    /// Held notes: (start beat, velocity).
    held: [Option<(f64, u8)>; 128],
}

pub struct Engine {
    sample_rate: u32,
    cmd_rx: rtrb::Consumer<EngineCommand>,
    evt_tx: rtrb::Producer<EngineEvent>,
    rec_tx: rtrb::Producer<f32>,
    meters: Arc<Meters>,
    input: Option<rtrb::Consumer<f32>>,
    armed: u64,
    project: Arc<Project>,
    tracks: Vec<TrackState>,
    audio: Vec<(u64, Arc<SampleData>)>,
    /// Effect chains, by track id.
    fx: Vec<(u64, Box<Chain>)>,
    limiter: Limiter,
    metronome: bool,
    click: metronome::Click,
    /// Samples of count-in still to go before `pending_record` starts.
    count_in: u64,
    count_total: u64,
    pending_record: Option<u64>,
    /// Beats to play round and round.
    loop_region: Option<(f64, f64)>,
    playing: bool,
    pos: u64,
    recording: Option<Recording>,
    scratch: [f32; CHUNK],
    scratch_r: [f32; CHUNK],
    input_buf: [f32; CHUNK],
    since_position: u32,
}

impl Engine {
    pub fn new(sample_rate: u32, link: EngineEnd) -> Engine {
        Engine {
            sample_rate,
            cmd_rx: link.commands,
            evt_tx: link.events,
            rec_tx: link.recorded,
            meters: link.meters,
            input: None,
            armed: 0,
            project: Arc::new(Project::default()),
            tracks: Vec::with_capacity(MAX_TRACKS),
            audio: Vec::with_capacity(MAX_AUDIO_CLIPS),
            fx: Vec::with_capacity(MAX_TRACKS),
            limiter: Limiter::new(sample_rate as f32),
            metronome: false,
            click: metronome::Click::new(sample_rate),
            count_in: 0,
            count_total: 0,
            pending_record: None,
            loop_region: None,
            playing: false,
            pos: 0,
            recording: None,
            scratch: [0.0; CHUNK],
            scratch_r: [0.0; CHUNK],
            input_buf: [0.0; CHUNK],
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

    fn synth_for(&mut self, track: u64) -> Option<&mut Instrument> {
        self.tracks
            .iter_mut()
            .find(|t| t.id == track)
            .map(|t| &mut t.instrument)
    }

    fn set_project(&mut self, p: Arc<Project>) {
        self.tracks.retain(|t| p.track(t.id).is_some());
        while let Some(i) = self.fx.iter().position(|(id, _)| p.track(*id).is_none()) {
            let (_, chain) = self.fx.swap_remove(i);
            self.send(EngineEvent::RetiredEffects(chain));
        }
        for (id, chain) in &mut self.fx {
            if let Some(t) = p.track(*id) {
                chain.configure(&t.effects);
            }
        }
        let sr = self.sample_rate;
        for track in &p.tracks {
            let Some(sound) = track.sound() else {
                continue;
            };
            let hash = sound_hash(sound);
            if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track.id) {
                // Built-in sounds are swapped here; anything heavier arrives later in a
                // SetInstrument, and the old instrument keeps playing until then.
                if t.sound != hash && t.instrument.is_heap_free() {
                    t.sound = hash;
                    t.instrument.all_notes_off();
                    t.instrument = Instrument::builtin(sound, sr).unwrap_or(Instrument::None);
                }
            } else if self.tracks.len() < MAX_TRACKS {
                self.tracks.push(TrackState {
                    id: track.id,
                    sound: hash,
                    instrument: Instrument::builtin(sound, sr).unwrap_or(Instrument::None),
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

    fn set_effects(&mut self, track: u64, mut chain: Box<Chain>) {
        let Some(t) = self.project.track(track) else {
            self.send(EngineEvent::RetiredEffects(chain));
            return;
        };
        chain.configure(&t.effects);
        let slot = self.fx.iter().position(|(id, _)| *id == track);
        let retired = match slot {
            Some(i) => Some(std::mem::replace(&mut self.fx[i].1, chain)),
            None if self.fx.len() < MAX_TRACKS => {
                self.fx.push((track, chain));
                None
            }
            None => Some(chain),
        };
        if let Some(old) = retired {
            self.send(EngineEvent::RetiredEffects(old));
        }
    }

    fn set_audio(&mut self, clip: u64, data: Option<Arc<SampleData>>) {
        let slot = self.audio.iter().position(|(id, _)| *id == clip);
        let retired = match (slot, data) {
            (Some(i), Some(new)) => Some(std::mem::replace(&mut self.audio[i].1, new)),
            (Some(i), None) => Some(self.audio.swap_remove(i).1),
            (None, Some(new)) if self.audio.len() < MAX_AUDIO_CLIPS => {
                self.audio.push((clip, new));
                None
            }
            (None, new) => new,
        };
        if let Some(old) = retired {
            self.send(EngineEvent::RetiredAudio(old));
        }
    }

    fn all_off(&mut self) {
        for t in &mut self.tracks {
            t.instrument.all_notes_off();
        }
    }

    fn begin_record(&mut self, track: u64) {
        let audio = self.project.track(track).is_some_and(Track::is_audio);
        // Note takes become whole-bar clips; a voice take starts right here.
        let start_beat = if audio {
            self.beat()
        } else {
            (self.beat() / BEATS_PER_BAR).floor() * BEATS_PER_BAR
        };
        self.recording = Some(Recording {
            track,
            start_beat,
            audio,
            held: [None; 128],
        });
        self.playing = true;
    }

    /// The loop as samples, while it applies (never during a take).
    fn active_loop(&self) -> Option<(u64, u64)> {
        let (from, to) = self.loop_region?;
        if !self.playing || self.recording.is_some() {
            return None;
        }
        let tempo = self.project.tempo_bpm;
        let start = beat_to_sample(from, tempo, self.sample_rate);
        let end = beat_to_sample(to, tempo, self.sample_rate);
        (end > start).then_some((start, end))
    }

    fn stop(&mut self) {
        let beat = self.beat();
        if let Some(track) = self.pending_record.take() {
            // Stopped during the count-in: the take is over before it began.
            self.count_in = 0;
            self.send(EngineEvent::RecordFinished {
                track,
                start_beat: beat,
                length_beats: 0.0,
            });
        }
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
            let length_beats = if rec.audio {
                (beat - rec.start_beat).max(0.0)
            } else {
                ((beat - rec.start_beat) / BEATS_PER_BAR).ceil().max(1.0) * BEATS_PER_BAR
            };
            self.send(EngineEvent::RecordFinished {
                track: rec.track,
                start_beat: rec.start_beat,
                length_beats,
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
            EngineCommand::SetInstrument {
                track,
                sound,
                mut instrument,
            } => {
                if let Some(t) = self.tracks.iter_mut().find(|t| t.id == track) {
                    t.instrument.all_notes_off();
                    std::mem::swap(&mut t.instrument, &mut *instrument);
                    t.sound = sound;
                }
                // Hand the old one (or the unused new one) back to be freed off-thread.
                self.send(EngineEvent::RetiredInstrument(instrument));
            }
            EngineCommand::SetAudio { clip, data } => self.set_audio(clip, data),
            EngineCommand::SetEffects { track, chain } => self.set_effects(track, chain),
            EngineCommand::SetInput(input) => {
                if let Some(old) = std::mem::replace(&mut self.input, input) {
                    self.send(EngineEvent::RetiredInput(old));
                }
            }
            EngineCommand::SetArmed(track) => self.armed = track,
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
            EngineCommand::Record { track, count_in } => {
                if self.recording.is_some() || self.pending_record.is_some() {
                    return;
                }
                if count_in {
                    let bar =
                        samples_per_beat(self.project.tempo_bpm, self.sample_rate) * BEATS_PER_BAR;
                    self.playing = false;
                    self.all_off();
                    self.pending_record = Some(track);
                    self.count_total = (bar.round() as u64).max(1);
                    self.count_in = self.count_total;
                } else {
                    self.begin_record(track);
                }
            }
            EngineCommand::SetMetronome(on) => self.metronome = on,
            EngineCommand::SetLoop(region) => self.loop_region = region,
            EngineCommand::Locate(beat) => {
                // Jumping mid-take would corrupt the recording.
                if self.recording.is_some() || self.pending_record.is_some() {
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
            // Chunks stop exactly where the count-in ends or the loop turns round.
            let mut n = CHUNK.min(frames - done);
            let turn = self.active_loop().filter(|(_, end)| self.pos < *end);
            if self.count_in > 0 {
                n = n.min(self.count_in as usize);
            } else if let Some((_, end)) = turn {
                n = n.min((end - self.pos) as usize);
            }
            self.render_chunk(
                &mut out[done * channels..(done + n) * channels],
                n,
                channels,
            );
            done += n;
            if let Some((start, end)) = turn
                && self.pos >= end
            {
                self.pos = start;
                self.all_off();
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// Fill `input_buf[..n]` with the next microphone samples (silence if none).
    fn pull_input(&mut self, n: usize) {
        let buf = &mut self.input_buf[..n];
        buf.fill(0.0);
        let Some(input) = &mut self.input else {
            return;
        };
        let backlog = input.slots();
        if backlog > n + MAX_INPUT_BACKLOG
            && let Ok(skip) = input.read_chunk(backlog - n - MAX_INPUT_BACKLOG / 2)
        {
            skip.commit_all();
        }
        let take = input.slots().min(n);
        if let Ok(chunk) = input.read_chunk(take) {
            let (a, b) = chunk.as_slices();
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();
        }
        let peak = buf.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        self.meters.raise_input(peak);
    }

    fn render_chunk(&mut self, out: &mut [f32], n: usize, channels: usize) {
        let project = Arc::clone(&self.project);
        let any_solo = project.tracks.iter().any(|t| t.solo);
        let (start, end) = (self.pos, self.pos + n as u64);
        let tempo = project.tempo_bpm;
        let sr = self.sample_rate;

        self.pull_input(n);
        if self.playing
            && let Some(rec) = &self.recording
            && rec.audio
        {
            for &s in &self.input_buf[..n] {
                // A full queue means the writer stalled; losing samples beats blocking.
                let _ = self.rec_tx.push(s);
            }
        }

        for (index, track) in project.tracks.iter().enumerate() {
            let scratch = &mut self.scratch[..n];
            scratch.fill(0.0);
            let scratch_r = &mut self.scratch_r[..n];
            scratch_r.fill(0.0);

            if track.is_audio() {
                if self.playing {
                    for clip in &track.clips {
                        let Some((_, data)) = self.audio.iter().find(|(id, _)| *id == clip.id)
                        else {
                            continue;
                        };
                        let clip_start = beat_to_sample(clip.start_beat, tempo, sr);
                        let clip_end =
                            beat_to_sample(clip.start_beat + clip.length_beats, tempo, sr);
                        let skip = (clip.audio_offset.max(0.0) * sr as f64) as u64;
                        mix_clip(
                            data, clip_start, clip_end, skip, start, sr, scratch, scratch_r,
                        );
                    }
                }
                if track.monitor && track.id == self.armed {
                    for (i, s) in self.input_buf[..n].iter().enumerate() {
                        scratch[i] += s;
                        scratch_r[i] += s;
                    }
                }
            } else {
                let Some(state) = self.tracks.iter_mut().find(|t| t.id == track.id) else {
                    continue;
                };
                render_notes(
                    track,
                    state,
                    self.playing,
                    start,
                    end,
                    tempo,
                    sr,
                    scratch,
                    scratch_r,
                );
            }

            // Effects run even while muted, so echoes and tails stay in step.
            if let Some((_, chain)) = self.fx.iter_mut().find(|(id, _)| *id == track.id) {
                chain.process(scratch, scratch_r);
            }

            let audible = !track.mute && (!any_solo || track.solo);
            if !audible {
                continue;
            }
            let angle = (track.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let (l, r) = (angle.cos() * track.volume, angle.sin() * track.volume);
            let mut loudest = 0.0f32;
            for (i, (sl, sr)) in scratch.iter().zip(scratch_r.iter()).enumerate() {
                loudest = loudest.max((sl * l).abs()).max((sr * r).abs());
                let frame = &mut out[i * channels..(i + 1) * channels];
                if channels == 1 {
                    frame[0] += (sl * l + sr * r) * 0.5;
                } else {
                    frame[0] += sl * l;
                    frame[1] += sr * r;
                }
            }
            self.meters.raise_track(index, loudest);
        }

        // Master: volume, then a limiter so a busy mix never clips.
        let volume = project.master_volume.clamp(0.0, 2.0);
        let mut peaks = [0.0f32; 2];
        for frame in out.chunks_exact_mut(channels) {
            let used = channels.min(2);
            for x in &mut frame[..used] {
                *x *= volume;
            }
            self.limiter.run(&mut frame[..used]);
            peaks[0] = peaks[0].max(frame[0].abs());
            peaks[1] = peaks[1].max(frame[used - 1].abs());
        }
        self.meters.raise_master(peaks);

        // The metronome is for the player, so it sits on top of the mix.
        let mut tick = None;
        if self.count_in > 0 {
            let beat = samples_per_beat(tempo, sr).round().max(1.0) as u64;
            let elapsed = self.count_total - self.count_in;
            let wait = (beat - elapsed % beat) % beat;
            if wait < n as u64 {
                tick = Some((wait as usize, elapsed + wait == 0));
            }
        } else if self.playing && self.metronome {
            let next = (sample_to_beat(start, tempo, sr) - 1e-9).ceil().max(0.0);
            let at = beat_to_sample(next, tempo, sr);
            if at >= start && at < end {
                tick = Some(((at - start) as usize, next % BEATS_PER_BAR == 0.0));
            }
        }
        if tick.is_some() || !self.click.is_silent() {
            for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                if let Some((at, accent)) = tick
                    && at == i
                {
                    self.click.trigger(accent);
                }
                let c = self.click.next();
                for x in &mut frame[..channels.min(2)] {
                    *x += c;
                }
            }
        }

        if self.count_in > 0 {
            self.count_in -= (n as u64).min(self.count_in);
            if self.count_in == 0
                && let Some(track) = self.pending_record.take()
            {
                self.begin_record(track);
            }
            return;
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

/// Play a track's notes into `left`/`right` for samples `start..end`, split at note
/// events so timing is sample-accurate.
#[allow(clippy::too_many_arguments)]
fn render_notes(
    track: &Track,
    state: &mut TrackState,
    playing: bool,
    start: u64,
    end: u64,
    tempo: f64,
    sr: u32,
    left: &mut [f32],
    right: &mut [f32],
) {
    if !playing {
        state.instrument.render(left, right);
        return;
    }
    let mut cur = start;
    loop {
        // Fire every event at exactly `cur`; find the next one after it. Note-offs
        // first, so a note ending where a same-pitch note starts doesn't cut the new one.
        let mut next = end;
        for pass_on in [false, true] {
            for clip in &track.clips {
                for note in &clip.notes {
                    let on_beat = clip.start_beat + note.start_beat;
                    let on = beat_to_sample(on_beat, tempo, sr);
                    // A note always lasts at least one sample, or it would never be
                    // switched off.
                    let off = beat_to_sample(on_beat + note.length_beats, tempo, sr).max(on + 1);
                    if pass_on {
                        if on == cur {
                            state.instrument.note_on(note.pitch, note.velocity);
                        }
                        for t in [on, off] {
                            if t > cur && t < next {
                                next = t;
                            }
                        }
                    } else if off == cur {
                        state.instrument.note_off(note.pitch);
                    }
                }
            }
        }
        let a = (cur - start) as usize;
        let b = (next - start) as usize;
        state.instrument.render(&mut left[a..b], &mut right[a..b]);
        if next >= end {
            break;
        }
        cur = next;
    }
}

/// Add the part of an audio clip (playing from sample `clip_start` until `clip_end`,
/// beginning `skip` samples into its audio) that falls in the block starting at
/// `block_start`.
#[allow(clippy::too_many_arguments)]
fn mix_clip(
    data: &SampleData,
    clip_start: u64,
    clip_end: u64,
    skip: u64,
    block_start: u64,
    sr: u32,
    left: &mut [f32],
    right: &mut [f32],
) {
    let block_end = block_start + left.len() as u64;
    if block_end <= clip_start || block_start >= clip_end {
        return;
    }
    let ratio = data.rate as f64 / sr as f64;
    let frames = data.frames.len() / 2;
    let first = clip_start.saturating_sub(block_start) as usize;
    let last = (clip_end.min(block_end) - block_start) as usize;
    for i in first..last {
        let local = block_start + i as u64 - clip_start + skip;
        let pos = local as f64 * ratio;
        let idx = pos as usize;
        if idx >= frames {
            break;
        }
        let next = (idx + 1).min(frames - 1);
        let frac = (pos - idx as f64) as f32;
        let (l0, r0) = (data.frames[idx * 2], data.frames[idx * 2 + 1]);
        let (l1, r1) = (data.frames[next * 2], data.frames[next * 2 + 1]);
        left[i] += l0 + (l1 - l0) * frac;
        right[i] += r0 + (r1 - r0) * frac;
    }
}

#[cfg(test)]
mod tests;
