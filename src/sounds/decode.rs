//! Decode audio files (WAV, FLAC, OGG, MP3) into stereo f32 sample data.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::engine::sampler::SampleData;

/// Longest file we will load into memory (about 10 minutes of stereo at 48 kHz).
const MAX_FRAMES: usize = 48_000 * 60 * 10;

/// Stereo f32, for recordings and audio clips.
pub fn decode_file(path: &Path) -> Result<SampleData, String> {
    let mut rate = 0;
    let mut frames: Vec<f32> = Vec::new();
    decode_with(path, |r, channels, buf| {
        rate = r;
        for frame in buf.chunks(channels) {
            let (l, r) = if channels == 1 {
                (frame[0], frame[0])
            } else {
                (frame[0], frame[1])
            };
            frames.push(l);
            frames.push(r);
        }
        frames.len() / 2
    })?;
    if frames.is_empty() || rate == 0 {
        return Err("the file contains no audio".into());
    }
    Ok(SampleData::stereo(rate, frames))
}

/// 16-bit for instrument samples; mono stays mono, more than two channels keep the first two.
pub fn decode_compact(path: &Path) -> Result<SampleData, String> {
    let mut rate = 0;
    let mut out_channels = 0usize;
    let mut data: Vec<i16> = Vec::new();
    decode_with(path, |r, channels, buf| {
        rate = r;
        if out_channels == 0 {
            out_channels = channels.min(2);
        }
        for frame in buf.chunks(channels) {
            for &x in &frame[..out_channels] {
                data.push((x * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
            }
        }
        data.len() / out_channels
    })?;
    if data.is_empty() || rate == 0 {
        return Err("the file contains no audio".into());
    }
    Ok(SampleData::compact(rate, out_channels as u8, data))
}

/// Feeds each decoded packet to `sink(rate, channels, interleaved)`; the sink returns
/// the frames collected so far, to enforce the length limit.
fn decode_with(
    path: &Path,
    mut sink: impl FnMut(u32, usize, &[f32]) -> usize,
) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("unsupported audio file ({e})"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio in this file")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("no audio in this file")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("unsupported codec ({e})"))?;
    let track_id = track.id;

    let mut buf: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(_)) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let channels = audio.spec().channels().count().max(1);
        buf.resize(audio.samples_interleaved(), 0.0);
        audio.copy_to_slice_interleaved(&mut buf);
        if sink(audio.spec().rate(), channels, &buf) > MAX_FRAMES {
            return Err("file is too long".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::sampler::Frames;

    #[test]
    fn decodes_mono_wav_to_stereo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 22_050,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..1000 {
            w.write_sample(if i % 2 == 0 { 16_000i16 } else { -16_000 })
                .unwrap();
        }
        w.finalize().unwrap();
        let d = decode_file(&path).unwrap();
        assert_eq!(d.rate, 22_050);
        assert_eq!(d.len(), 1000);
        let (l, r) = d.frame(0);
        assert!((l - 0.488).abs() < 0.01);
        assert_eq!(l, r);
    }

    #[test]
    fn decode_compact_keeps_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 22_050,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..1000 {
            w.write_sample(16_000i16).unwrap();
        }
        w.finalize().unwrap();
        let d = decode_compact(&path).unwrap();
        assert!(matches!(d.frames, Frames::Compact { channels: 1, .. }));
        assert_eq!(d.len(), 1000);
        assert_eq!(d.rate, 22_050);
    }

    #[test]
    fn decode_compact_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..10 {
            w.write_sample(1.5f32).unwrap();
        }
        w.finalize().unwrap();
        let d = decode_compact(&path).unwrap();
        let Frames::Compact { data, .. } = &d.frames else {
            panic!("not compact")
        };
        assert_eq!(data[0], i16::MAX);
    }

    #[test]
    fn rejects_non_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        std::fs::write(&path, b"not audio at all").unwrap();
        assert!(decode_file(&path).is_err());
    }
}
