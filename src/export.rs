//! Saving the song as an audio file. The song is played through a private engine as
//! fast as the computer can go, so what you get is exactly what you hear.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use crate::engine::Engine;
use crate::engine::commands::{EngineCommand, link};
use crate::engine::fx::Chain;
use crate::engine::instrument::sound_hash;
use crate::engine::transport::beat_to_sample;
use crate::project::Project;

/// Sample rate of exported files: what CDs, streaming services and MP3 use.
pub const EXPORT_RATE: u32 = 44_100;
const MP3_KBPS: u32 = 192;
const BLOCK_FRAMES: usize = 1024;
/// After the last clip, keep going until echoes and reverb have died away...
const SILENCE: f32 = 1e-4;
/// (longer than the longest echo time, so a gap between echoes isn't taken for the end)
const SILENT_SECONDS: f64 = 2.0;
/// ...but never longer than this.
const MAX_TAIL_SECONDS: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Wav,
    Mp3,
}

impl Format {
    /// Chosen by the file's extension; anything unknown is a WAV.
    pub fn of(path: &Path) -> Format {
        match path.extension().and_then(|e| e.to_str()) {
            Some(e) if e.eq_ignore_ascii_case("mp3") => Format::Mp3,
            _ => Format::Wav,
        }
    }
}

/// Play the whole song into memory: interleaved stereo at `sample_rate`.
pub fn render(
    project: &Project,
    audio_dir: &Path,
    sample_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<f32>, String> {
    let end_beat = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| c.end_beat())
        .fold(0.0, f64::max);
    if end_beat <= 0.0 {
        return Err("There is nothing to export yet. Record or draw something first.".into());
    }

    let (mut ui, engine_end) = link();
    let mut engine = Engine::new(sample_rate, engine_end);
    let mut send = |engine: &mut Engine, cmd: EngineCommand| {
        if ui.commands.push(cmd).is_err() {
            return Err("The export engine stopped responding.".to_string());
        }
        // Let the engine take the command in (and hand back what it replaced).
        engine.process(&mut [], 2);
        while ui.events.pop().is_ok() {}
        Ok(())
    };
    send(
        &mut engine,
        EngineCommand::SetProject(Arc::new(project.clone())),
    )?;
    for track in &project.tracks {
        if let Some(sound) = track.sound() {
            let instrument = crate::sounds::load(sound, sample_rate)
                .map_err(|e| format!("Could not load the sound of \"{}\": {e}", track.name))?;
            send(
                &mut engine,
                EngineCommand::SetInstrument {
                    track: track.id,
                    sound: sound_hash(sound),
                    instrument: Box::new(instrument),
                },
            )?;
        }
        if !track.effects.is_empty() {
            send(
                &mut engine,
                EngineCommand::SetEffects {
                    track: track.id,
                    chain: Box::new(Chain::build(&track.effects, sample_rate)),
                },
            )?;
        }
        for clip in &track.clips {
            let Some(file) = &clip.audio else {
                continue;
            };
            let data = crate::sounds::decode::decode_file(&audio_dir.join(file))
                .map_err(|e| format!("Could not read the recording \"{file}\": {e}"))?;
            send(
                &mut engine,
                EngineCommand::SetAudio {
                    clip: clip.id,
                    data: Some(Arc::new(data)),
                },
            )?;
        }
    }
    send(&mut engine, EngineCommand::Play)?;

    let song = beat_to_sample(end_beat, project.tempo_bpm, sample_rate) as usize;
    let longest = song + (MAX_TAIL_SECONDS * sample_rate as f64) as usize;
    let quiet_needed = (SILENT_SECONDS * sample_rate as f64) as usize;
    let mut out = Vec::with_capacity((song + sample_rate as usize) * 2);
    let mut block = vec![0.0f32; BLOCK_FRAMES * 2];
    let mut quiet = 0;
    loop {
        engine.process(&mut block, 2);
        while ui.events.pop().is_ok() {}
        out.extend_from_slice(&block);
        let frames = out.len() / 2;
        if block.iter().all(|x| x.abs() < SILENCE) {
            quiet += BLOCK_FRAMES;
        } else {
            quiet = 0;
        }
        if frames >= longest || (frames >= song && quiet >= quiet_needed) {
            break;
        }
        progress((frames as f32 / song.max(1) as f32).min(1.0));
    }
    // Drop the silence we waited through, keeping a short breath at the end.
    let keep = (out.len() / 2).saturating_sub(quiet.saturating_sub(BLOCK_FRAMES));
    out.truncate(keep.max(song.min(out.len() / 2)) * 2);
    progress(1.0);
    Ok(out)
}

fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for s in samples {
        w.write_sample(to_i16(*s)).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

pub fn encode_mp3(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    use shine_rs::{Mp3Encoder, Mp3EncoderConfig, StereoMode};
    let config = Mp3EncoderConfig::new()
        .sample_rate(sample_rate)
        .bitrate(MP3_KBPS)
        .channels(2)
        .stereo_mode(StereoMode::JointStereo);
    let mut encoder = Mp3Encoder::new(config).map_err(|e| e.to_string())?;
    let pcm: Vec<i16> = samples.iter().map(|s| to_i16(*s)).collect();
    let mut mp3 = Vec::with_capacity(pcm.len() / 6);
    for frame in encoder
        .encode_interleaved(&pcm)
        .map_err(|e| e.to_string())?
    {
        mp3.extend_from_slice(&frame);
    }
    mp3.extend_from_slice(&encoder.finish().map_err(|e| e.to_string())?);
    Ok(mp3)
}

/// Render the song and save it at `path`, as the format its extension asks for.
pub fn export(
    project: &Project,
    audio_dir: &Path,
    path: &Path,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    // Rendering is nearly all of the work; encoding gets the last bit of the bar.
    let samples = render(project, audio_dir, EXPORT_RATE, &mut |p| progress(p * 0.9))?;
    match Format::of(path) {
        Format::Wav => write_wav(path, &samples, EXPORT_RATE)?,
        Format::Mp3 => {
            let mp3 = encode_mp3(&samples, EXPORT_RATE)?;
            std::fs::write(path, mp3).map_err(|e| e.to_string())?;
        }
    }
    progress(1.0);
    Ok(())
}

pub enum Update {
    Progress(f32),
    Done(Result<PathBuf, String>),
}

/// Export on a background thread; `wake` is called whenever there is news.
pub fn start(
    project: Project,
    audio_dir: PathBuf,
    path: PathBuf,
    wake: impl Fn() + Send + 'static,
) -> mpsc::Receiver<Update> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            let mut last = -1.0f32;
            let result = export(&project, &audio_dir, &path, &mut |p| {
                if p - last >= 0.01 {
                    last = p;
                    let _ = tx.send(Update::Progress(p));
                    wake();
                }
            });
            let _ = tx.send(Update::Done(result.map(|()| path)));
            wake();
        })
        .expect("spawn export thread");
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::effects::{Effect, EffectKind};
    use crate::project::{Clip, Note};

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    /// One Keys track with a one-beat note at beat 1 of a four-beat clip.
    fn song() -> Project {
        let mut p = Project::default();
        let track = p.add_instrument_track("Keys");
        let clip = p.add_empty_clip(track, 0.0, 4.0).unwrap();
        p.clip_mut(clip).unwrap().notes = vec![Note {
            pitch: 60,
            velocity: 100,
            start_beat: 1.0,
            length_beats: 1.0,
        }];
        p
    }

    #[test]
    fn format_follows_the_extension() {
        assert_eq!(Format::of(Path::new("song.mp3")), Format::Mp3);
        assert_eq!(Format::of(Path::new("song.MP3")), Format::Mp3);
        assert_eq!(Format::of(Path::new("song.wav")), Format::Wav);
        assert_eq!(Format::of(Path::new("song")), Format::Wav);
    }

    #[test]
    fn empty_song_is_refused() {
        let mut p = Project::default();
        p.add_instrument_track("Keys");
        let err = render(&p, Path::new("."), 44_100, &mut |_| {}).unwrap_err();
        assert!(err.contains("nothing to export"));
    }

    #[test]
    fn render_plays_the_song_from_start_to_end() {
        let mut steps = Vec::new();
        let out = render(&song(), Path::new("."), 48_000, &mut |p| steps.push(p)).unwrap();
        let frames = out.len() / 2;
        // Four beats at 120 BPM is two seconds; the note starts half a second in.
        assert!((96_000..96_000 + 48_000).contains(&frames), "{frames}");
        assert_eq!(peak(&out[..24_000 * 2]), 0.0);
        assert!(peak(&out[24_000 * 2..48_000 * 2]) > 0.01);
        assert_eq!(steps.last(), Some(&1.0));
        assert!(steps.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn render_waits_for_effect_tails() {
        let mut p = song();
        // The note fills the last beat; its echo a second later outlasts the clip.
        p.clip_mut(p.tracks[0].clips[0].id).unwrap().notes[0].start_beat = 3.0;
        p.tracks[0].effects = vec![Effect::with(
            EffectKind::Delay,
            &[(0, 1000.0), (1, 0.0), (2, 1.0)],
        )];
        let out = render(&p, Path::new("."), 48_000, &mut |_| {}).unwrap();
        let frames = out.len() / 2;
        assert!(frames > 96_000 + 48_000, "tail was cut: {frames}");
        assert!(peak(&out[(96_000 + 30_000) * 2..]) > 0.005);
    }

    #[test]
    fn render_includes_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(dir.path().join("take.wav"), spec).unwrap();
        for _ in 0..24_000 {
            w.write_sample(8000i16).unwrap();
        }
        w.finalize().unwrap();
        let mut p = Project::default();
        let track = p.add_voice_track();
        let id = p.new_id();
        p.track_mut(track).unwrap().clips.push(Clip {
            id,
            start_beat: 1.0,
            length_beats: 1.0,
            audio: Some("take.wav".into()),
            ..Clip::default()
        });
        let out = render(&p, dir.path(), 48_000, &mut |_| {}).unwrap();
        assert_eq!(peak(&out[..24_000 * 2]), 0.0);
        assert!(peak(&out[24_000 * 2..48_000 * 2]) > 0.05);

        p.tracks[0].clips[0].audio = Some("missing.wav".into());
        let err = render(&p, dir.path(), 48_000, &mut |_| {}).unwrap_err();
        assert!(err.contains("missing.wav"), "{err}");
    }

    #[test]
    fn wav_export_can_be_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.wav");
        export(&song(), Path::new("."), &path, &mut |_| {}).unwrap();
        let mut r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.spec().sample_rate, EXPORT_RATE);
        assert_eq!(r.spec().channels, 2);
        let samples: Vec<i16> = r.samples::<i16>().map(Result::unwrap).collect();
        assert!(samples.len() >= 88_200 * 2);
        assert!(samples.iter().any(|s| s.abs() > 300));
    }

    #[test]
    fn mp3_export_is_a_playable_file_of_the_right_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.mp3");
        export(&song(), Path::new("."), &path, &mut |_| {}).unwrap();
        let decoded = crate::sounds::decode::decode_file(&path).unwrap();
        let crate::engine::sampler::Frames::Stereo(frames) = &decoded.frames else {
            panic!("decode_file returns stereo")
        };
        assert_eq!(decoded.rate, EXPORT_RATE);
        let seconds = decoded.len() as f64 / EXPORT_RATE as f64;
        assert!((2.0..3.6).contains(&seconds), "{seconds} s");
        // Silent before the note, sounding during it.
        assert!(peak(&frames[..20_000 * 2]) < 0.01);
        assert!(peak(&frames[24_000 * 2..40_000 * 2]) > 0.01);
    }

    #[test]
    fn background_export_reports_progress_then_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.wav");
        let rx = start(song(), PathBuf::from("."), path.clone(), || {});
        let mut saw_progress = false;
        loop {
            match rx.recv().unwrap() {
                Update::Progress(p) => {
                    assert!((0.0..=1.0).contains(&p));
                    saw_progress = true;
                }
                Update::Done(result) => {
                    assert_eq!(result.unwrap(), path);
                    break;
                }
            }
        }
        assert!(saw_progress);
        assert!(path.is_file());
    }
}
