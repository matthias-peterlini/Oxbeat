use super::*;
use crate::project::Clip;

struct Rig {
    engine: Engine,
    cmd: rtrb::Producer<EngineCommand>,
    evt: rtrb::Consumer<EngineEvent>,
}

fn rig(sr: u32) -> Rig {
    let (cmd, cmd_rx) = rtrb::RingBuffer::new(1024);
    let (evt_tx, evt) = rtrb::RingBuffer::new(8192);
    Rig {
        engine: Engine::new(sr, cmd_rx, evt_tx),
        cmd,
        evt,
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
