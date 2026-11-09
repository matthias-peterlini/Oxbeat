use super::*;
use crate::project::Clip;

struct Rig {
    engine: Engine,
    cmd: rtrb::Producer<EngineCommand>,
    evt: rtrb::Consumer<EngineEvent>,
    recorded: rtrb::Consumer<f32>,
    meters: Arc<Meters>,
}

fn rig(sr: u32) -> Rig {
    let (ui, end) = commands::link();
    Rig {
        engine: Engine::new(sr, end),
        cmd: ui.commands,
        evt: ui.events,
        recorded: ui.recorded,
        meters: ui.meters,
    }
}

impl Rig {
    fn send(&mut self, c: EngineCommand) {
        self.cmd.push(c).ok().unwrap();
    }
    /// Render `frames` stereo frames in blocks of `block`; returns left channel.
    fn render_blocks(&mut self, frames: usize, block: usize) -> Vec<f32> {
        let mut left = Vec::with_capacity(frames);
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            let mut buf = vec![0.0f32; n * 2];
            self.engine.process(&mut buf, 2);
            left.extend(buf.chunks(2).map(|f| f[0]));
            done += n;
        }
        left
    }
    fn render(&mut self, frames: usize) -> Vec<f32> {
        self.render_blocks(frames, 512)
    }
    fn events(&mut self) -> Vec<EngineEvent> {
        let mut v = Vec::new();
        while let Ok(e) = self.evt.pop() {
            v.push(e);
        }
        v
    }
}

fn first_nonzero(v: &[f32]) -> Option<usize> {
    v.iter().position(|x| *x != 0.0)
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

/// Project with `n` Keys tracks; returns (project, track ids).
fn project(n: usize) -> (Project, Vec<u64>) {
    let mut p = Project::default();
    let ids = (0..n).map(|_| p.add_instrument_track("Keys")).collect();
    (p, ids)
}

fn with_note_at_beat_1(p: &mut Project, track: u64) {
    let id = p.new_id();
    p.track_mut(track).unwrap().clips.push(Clip {
        id,
        start_beat: 0.0,
        length_beats: 4.0,
        notes: vec![Note {
            pitch: 60,
            velocity: 100,
            start_beat: 1.0,
            length_beats: 1.0,
        }],
        audio: None,
    });
}

#[test]
fn live_note_sounds_when_stopped() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::NoteOn {
        track: ids[0],
        pitch: 60,
        velocity: 100,
    });
    assert!(peak(&r.render(4800)) > 0.01);
}

#[test]
fn muted_track_is_silent() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(1);
    p.tracks[0].mute = true;
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::NoteOn {
        track: ids[0],
        pitch: 60,
        velocity: 100,
    });
    assert_eq!(peak(&r.render(4800)), 0.0);
}

#[test]
fn solo_silences_others() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(2);
    p.tracks[1].solo = true;
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::NoteOn {
        track: ids[0],
        pitch: 60,
        velocity: 100,
    });
    assert_eq!(peak(&r.render(4800)), 0.0);
    r.send(EngineCommand::NoteOn {
        track: ids[1],
        pitch: 60,
        velocity: 100,
    });
    assert!(peak(&r.render(4800)) > 0.01);
}

#[test]
fn clip_note_starts_on_exact_sample() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(1);
    with_note_at_beat_1(&mut p, ids[0]);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Play);
    let out = r.render(30_000);
    assert_eq!(first_nonzero(&out), Some(24_000));
}

#[test]
fn timing_correct_at_44100() {
    let mut r = rig(44_100);
    let (mut p, ids) = project(1);
    with_note_at_beat_1(&mut p, ids[0]);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Play);
    let out = r.render(30_000);
    assert_eq!(first_nonzero(&out), Some(22_050));
}

fn recorded(events: Vec<EngineEvent>) -> (Vec<Note>, Option<(f64, f64)>) {
    let mut notes = Vec::new();
    let mut fin = None;
    for e in events {
        match e {
            EngineEvent::RecordedNote { note, .. } => notes.push(note),
            EngineEvent::RecordFinished {
                start_beat,
                length_beats,
                ..
            } => fin = Some((start_beat, length_beats)),
            _ => {}
        }
    }
    (notes, fin)
}

#[test]
fn record_captures_note_in_beats() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    let t = ids[0];
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Record { track: t });
    r.render(24_000);
    r.send(EngineCommand::NoteOn {
        track: t,
        pitch: 62,
        velocity: 90,
    });
    r.render(24_000);
    r.send(EngineCommand::NoteOff {
        track: t,
        pitch: 62,
    });
    r.render(512);
    r.send(EngineCommand::Stop);
    r.render(512);
    let (notes, fin) = recorded(r.events());
    assert_eq!(
        notes,
        vec![Note {
            pitch: 62,
            velocity: 90,
            start_beat: 1.0,
            length_beats: 1.0
        }]
    );
    assert_eq!(fin, Some((0.0, 4.0)));
}

#[test]
fn held_note_closed_on_stop() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    let t = ids[0];
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Record { track: t });
    r.send(EngineCommand::NoteOn {
        track: t,
        pitch: 60,
        velocity: 100,
    });
    r.render(48_000);
    r.send(EngineCommand::Stop);
    r.render(512);
    let (notes, fin) = recorded(r.events());
    assert_eq!(notes.len(), 1);
    assert!((notes[0].length_beats - 2.0).abs() < 1e-9);
    assert_eq!(fin, Some((0.0, 4.0)));
}

#[test]
fn recording_longer_than_a_bar_rounds_up() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Record { track: ids[0] });
    r.render(24_000 * 5); // 5 beats
    r.send(EngineCommand::Stop);
    r.render(512);
    assert_eq!(recorded(r.events()).1, Some((0.0, 8.0)));
}

#[test]
fn all_notes_off_stops_live_notes() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    for pitch in [60, 64, 67] {
        r.send(EngineCommand::NoteOn {
            track: ids[0],
            pitch,
            velocity: 100,
        });
    }
    r.render(4800);
    r.send(EngineCommand::AllNotesOff { track: ids[0] });
    r.render(48_000);
    assert_eq!(peak(&r.render(512)), 0.0);
}

#[test]
fn block_size_independent() {
    let make = || {
        let mut r = rig(48_000);
        let (mut p, ids) = project(1);
        with_note_at_beat_1(&mut p, ids[0]);
        r.send(EngineCommand::SetProject(Arc::new(p)));
        r.send(EngineCommand::Play);
        r
    };
    let reference = make().render_blocks(96_000, 96_000);
    assert!(peak(&reference) > 0.01);
    for block in [1, 37, 4096] {
        assert_eq!(
            make().render_blocks(96_000, block),
            reference,
            "block {block}"
        );
    }
}

#[test]
fn output_is_clamped() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(4);
    for t in &mut p.tracks {
        t.volume = 1.5;
    }
    r.send(EngineCommand::SetProject(Arc::new(p)));
    for t in &ids {
        for pitch in 40..56 {
            r.send(EngineCommand::NoteOn {
                track: *t,
                pitch,
                velocity: 127,
            });
        }
    }
    let out = r.render(48_000);
    assert!(out.iter().all(|x| x.abs() <= 1.0));
}

#[test]
fn retired_projects_are_handed_back() {
    let mut r = rig(48_000);
    r.send(EngineCommand::SetProject(Arc::new(Project::default())));
    r.render(64);
    assert!(
        r.events()
            .iter()
            .any(|e| matches!(e, EngineEvent::Retired(_)))
    );
}

#[test]
fn zero_length_note_releases() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(1);
    with_note_at_beat_1(&mut p, ids[0]);
    p.tracks[0].clips[0].notes[0].length_beats = 0.0;
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Play);
    r.render(24_000 + 48_000);
    assert_eq!(peak(&r.render(512)), 0.0);
}

#[test]
fn same_block_tap_records_positive_length() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    let t = ids[0];
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Record { track: t });
    r.send(EngineCommand::NoteOn {
        track: t,
        pitch: 60,
        velocity: 100,
    });
    r.send(EngineCommand::NoteOff {
        track: t,
        pitch: 60,
    });
    r.render(512);
    r.send(EngineCommand::Stop);
    r.render(512);
    let (notes, _) = recorded(r.events());
    assert_eq!(notes.len(), 1);
    assert!(notes[0].length_beats > 0.0);
}

#[test]
fn locate_ignored_while_recording() {
    let mut r = rig(48_000);
    let (p, ids) = project(1);
    let t = ids[0];
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Record { track: t });
    r.render(48_000);
    r.send(EngineCommand::NoteOn {
        track: t,
        pitch: 60,
        velocity: 100,
    });
    r.send(EngineCommand::Locate(0.0));
    r.render(24_000);
    r.send(EngineCommand::Stop);
    r.render(512);
    let (notes, _) = recorded(r.events());
    assert!(
        notes
            .iter()
            .all(|n| n.start_beat >= 0.0 && n.length_beats > 0.0)
    );
    assert!((notes[0].start_beat - 2.0).abs() < 0.05);
}

#[test]
fn tempo_change_keeps_beat_position() {
    let mut r = rig(48_000);
    let (mut p, _) = project(1);
    r.send(EngineCommand::SetProject(Arc::new(p.clone())));
    r.send(EngineCommand::Locate(16.0));
    r.render(64);
    p.tempo_bpm = 60.0;
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.render(64);
    r.events();
    r.send(EngineCommand::Stop);
    r.render(64);
    let pos = r
        .events()
        .into_iter()
        .rev()
        .find_map(|e| match e {
            EngineEvent::Position(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert!((pos - 16.0).abs() < 1e-6, "playhead at {pos}");
}

#[test]
fn tempo_change_releases_sounding_clip_notes() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(1);
    with_note_at_beat_1(&mut p, ids[0]);
    r.send(EngineCommand::SetProject(Arc::new(p.clone())));
    r.send(EngineCommand::Play);
    r.render(30_000); // note sounding (beat 1.25)
    p.tempo_bpm = 240.0;
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.render(48_000);
    assert_eq!(peak(&r.render(512)), 0.0);
}

/// A voice track with one audio clip of `frames` constant samples at beat `at`.
fn voice_with_clip(at: f64, frames: usize, rate: u32) -> (Project, u64, u64, Arc<SampleData>) {
    let mut p = Project::default();
    let track = p.add_voice_track();
    p.track_mut(track).unwrap().monitor = false;
    let clip = p.new_id();
    let data = Arc::new(SampleData {
        rate,
        frames: vec![0.5; frames * 2],
    });
    p.track_mut(track).unwrap().clips.push(Clip {
        id: clip,
        start_beat: at,
        length_beats: 64.0,
        notes: Vec::new(),
        audio: Some("take.wav".into()),
    });
    (p, track, clip, data)
}

#[test]
fn audio_clip_plays_from_its_start_beat() {
    let mut r = rig(48_000);
    let (p, _, clip, data) = voice_with_clip(1.0, 48_000, 48_000);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::SetAudio {
        clip,
        data: Some(data),
    });
    r.send(EngineCommand::Play);
    let out = r.render(48_000);
    // Beat 1 at 120 bpm is sample 24 000.
    assert_eq!(first_nonzero(&out), Some(24_000));
    assert!(peak(&out) > 0.2);
}

#[test]
fn audio_clip_is_resampled_to_the_engine_rate() {
    let mut r = rig(48_000);
    // One second of audio at 24 kHz lasts one second (48 000 samples) here.
    let (p, _, clip, data) = voice_with_clip(0.0, 24_000, 24_000);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::SetAudio {
        clip,
        data: Some(data),
    });
    r.send(EngineCommand::Play);
    let out = r.render(60_000);
    let last = out.iter().rposition(|x| *x != 0.0).unwrap();
    assert!((47_990..48_000).contains(&last), "ended at {last}");
}

#[test]
fn forgotten_audio_is_handed_back_and_silent() {
    let mut r = rig(48_000);
    let (p, _, clip, data) = voice_with_clip(0.0, 48_000, 48_000);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::SetAudio {
        clip,
        data: Some(data),
    });
    r.send(EngineCommand::SetAudio { clip, data: None });
    r.send(EngineCommand::Play);
    assert_eq!(peak(&r.render(4096)), 0.0);
    assert!(
        r.events()
            .iter()
            .any(|e| matches!(e, EngineEvent::RetiredAudio(_)))
    );
}

/// A microphone queue pre-filled with `n` samples of `value`.
fn mic(n: usize, value: f32) -> rtrb::Consumer<f32> {
    let (mut tx, rx) = rtrb::RingBuffer::new(n.max(1));
    for _ in 0..n {
        tx.push(value).unwrap();
    }
    rx
}

#[test]
fn voice_take_records_microphone_from_the_playhead() {
    let mut r = rig(48_000);
    let mut p = Project::default();
    let track = p.add_voice_track();
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::Locate(2.5));
    r.render(64);
    r.send(EngineCommand::SetInput(Some(mic(4096, 0.25))));
    r.send(EngineCommand::Record { track });
    r.render(1024);
    r.send(EngineCommand::Stop);
    r.render(64);
    let mut got = Vec::new();
    while let Ok(s) = r.recorded.pop() {
        got.push(s);
    }
    assert_eq!(got.len(), 1024);
    assert!(got.iter().all(|s| *s == 0.25));
    let finished = r.events().into_iter().find_map(|e| match e {
        EngineEvent::RecordFinished {
            start_beat,
            length_beats,
            ..
        } => Some((start_beat, length_beats)),
        _ => None,
    });
    let (start, len) = finished.unwrap();
    assert!(
        (start - 2.5).abs() < 1e-9,
        "voice takes start where recording began"
    );
    assert!((len - 1024.0 / 24_000.0).abs() < 1e-9);
    assert!(r.meters.take_input() >= 0.25);
}

#[test]
fn microphone_is_heard_only_on_the_armed_monitored_track() {
    let mut r = rig(48_000);
    let mut p = Project::default();
    let a = p.add_voice_track();
    let b = p.add_voice_track();
    p.track_mut(b).unwrap().monitor = false;
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::SetInput(Some(mic(8192, 0.5))));
    r.send(EngineCommand::SetArmed(b));
    assert_eq!(peak(&r.render(512)), 0.0, "b is armed but not monitored");
    r.send(EngineCommand::SetArmed(a));
    let out = r.render(512);
    assert!(peak(&out) > 0.2, "a is armed and monitored");
    // Nothing reaches the take queue unless recording.
    assert!(r.recorded.pop().is_err());
}

#[test]
fn closing_the_microphone_hands_the_queue_back() {
    let mut r = rig(48_000);
    r.send(EngineCommand::SetInput(Some(mic(16, 0.1))));
    r.send(EngineCommand::SetInput(None));
    r.render(64);
    assert!(
        r.events()
            .iter()
            .any(|e| matches!(e, EngineEvent::RetiredInput(_)))
    );
}

// ---- Effects, master and meters ----

use crate::project::effects::{Effect, EffectKind};

fn one_second_echo() -> Vec<Effect> {
    vec![Effect::with(
        EffectKind::Delay,
        &[(0, 1000.0), (1, 0.0), (2, 1.0)],
    )]
}

#[test]
fn track_effects_are_heard() {
    let render = |effects: Vec<Effect>| {
        let mut r = rig(48_000);
        let (mut p, ids) = project(1);
        with_note_at_beat_1(&mut p, ids[0]);
        p.tracks[0].effects = effects;
        let chain = fx::Chain::build(&p.tracks[0].effects, 48_000);
        r.send(EngineCommand::SetProject(Arc::new(p)));
        r.send(EngineCommand::SetEffects {
            track: ids[0],
            chain: Box::new(chain),
        });
        r.send(EngineCommand::Play);
        r.render(120_000)
    };
    // The note plays from sample 24 000 to 48 000; its echo arrives a second later.
    let dry = render(Vec::new());
    let wet = render(one_second_echo());
    assert!(peak(&dry[84_000..]) < 1e-3, "note still ringing");
    assert!(peak(&wet[84_000..96_000]) > 0.01, "no echo");
    assert_eq!(dry[..48_000], wet[..48_000]);
}

#[test]
fn knob_changes_arrive_with_the_project() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(1);
    p.tracks[0].effects = vec![Effect::with(EffectKind::Eq, &[(1, 0.0)])];
    let chain = fx::Chain::build(&p.tracks[0].effects, 48_000);
    r.send(EngineCommand::SetProject(Arc::new(p.clone())));
    r.send(EngineCommand::SetEffects {
        track: ids[0],
        chain: Box::new(chain),
    });
    r.send(EngineCommand::NoteOn {
        track: ids[0],
        pitch: 60,
        velocity: 100,
    });
    let flat = peak(&r.render(24_000)[12_000..]);
    // Turn the low shelf all the way down: middle C loses level.
    p.tracks[0].effects[0].set(0, -15.0);
    p.tracks[0].effects[0].set(1, -15.0);
    p.tracks[0].effects[0].set(2, 262.0);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    let cut = peak(&r.render(24_000)[12_000..]);
    assert!(cut < flat * 0.7, "{cut} vs {flat}");
}

#[test]
fn chains_come_back_when_replaced_or_their_track_is_deleted() {
    let retired = |r: &mut Rig| {
        r.events()
            .iter()
            .filter(|e| matches!(e, EngineEvent::RetiredEffects(_)))
            .count()
    };
    let mut r = rig(48_000);
    let (mut p, ids) = project(1);
    p.tracks[0].effects = one_second_echo();
    let build = |p: &Project| Box::new(fx::Chain::build(&p.tracks[0].effects, 48_000));
    r.send(EngineCommand::SetProject(Arc::new(p.clone())));
    r.send(EngineCommand::SetEffects {
        track: ids[0],
        chain: build(&p),
    });
    r.render(64);
    assert_eq!(retired(&mut r), 0);
    r.send(EngineCommand::SetEffects {
        track: ids[0],
        chain: build(&p),
    });
    r.render(64);
    assert_eq!(retired(&mut r), 1);
    // A chain for a track that doesn't exist goes straight back.
    r.send(EngineCommand::SetEffects {
        track: 999,
        chain: build(&p),
    });
    r.render(64);
    assert_eq!(retired(&mut r), 1);
    r.send(EngineCommand::SetProject(Arc::new(Project::default())));
    r.render(64);
    assert_eq!(retired(&mut r), 1);
}

#[test]
fn master_limiter_stops_a_loud_mix_from_clipping() {
    let mut r = rig(48_000);
    let (mut p, ids) = project(8);
    for t in &mut p.tracks {
        t.volume = 1.5;
        t.pan = -1.0;
    }
    r.send(EngineCommand::SetProject(Arc::new(p)));
    for id in &ids {
        r.send(EngineCommand::NoteOn {
            track: *id,
            pitch: 60,
            velocity: 127,
        });
    }
    let out = r.render(24_000);
    let loudest = peak(&out);
    assert!(loudest <= fx::dynamics::CEILING + 1e-6, "{loudest}");
    assert!(
        loudest > 0.9,
        "the mix should be pushing the limiter: {loudest}"
    );
}

#[test]
fn master_volume_scales_the_mix() {
    let level = |volume: f32| {
        let mut r = rig(48_000);
        let (mut p, ids) = project(1);
        p.master_volume = volume;
        r.send(EngineCommand::SetProject(Arc::new(p)));
        r.send(EngineCommand::NoteOn {
            track: ids[0],
            pitch: 60,
            velocity: 100,
        });
        peak(&r.render(9600))
    };
    assert_eq!(level(0.0), 0.0);
    let (half, full) = (level(0.5), level(1.0));
    assert!((half * 2.0 - full).abs() < 1e-4, "{half} {full}");
}

#[test]
fn meters_report_track_and_master_levels() {
    let mut r = rig(48_000);
    let (p, ids) = project(2);
    r.send(EngineCommand::SetProject(Arc::new(p)));
    r.send(EngineCommand::NoteOn {
        track: ids[1],
        pitch: 60,
        velocity: 100,
    });
    let out = r.render(9600);
    assert_eq!(r.meters.take_track(0), 0.0);
    assert!(r.meters.take_track(1) > 0.01);
    let [left, right] = r.meters.take_master();
    assert!((left - peak(&out)).abs() < 1e-6);
    assert!(right > 0.01);
    // Reading a meter resets it.
    assert_eq!(r.meters.take_master(), [0.0, 0.0]);
}
