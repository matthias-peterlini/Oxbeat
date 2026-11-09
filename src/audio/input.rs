//! Opens the default microphone and streams it, as mono samples at the engine's
//! sample rate, into a queue the engine reads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Room for about 1 s of microphone audio between the two device clocks.
const QUEUE_SECONDS: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("No microphone found. Plug one in, then select the voice track again.")]
    NoDevice,
    #[error("Could not open the microphone: {0}")]
    Build(String),
}

pub struct AudioInput {
    _stream: cpal::Stream,
    name: String,
    failed: Arc<AtomicBool>,
}

impl AudioInput {
    /// The microphone's name, as the system shows it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True once the device reported a fatal error (e.g. unplugged).
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// Turns interleaved frames at one rate into mono samples at another (linear
/// interpolation; plenty for a voice and cheap enough for the device thread).
pub struct ToMono {
    channels: usize,
    step: f64,
    pos: f64,
    prev: f32,
}

impl ToMono {
    pub fn new(channels: usize, from_rate: u32, to_rate: u32) -> Self {
        Self {
            channels: channels.max(1),
            step: from_rate as f64 / to_rate as f64,
            pos: 1.0,
            prev: 0.0,
        }
    }

    pub fn push(&mut self, frames: &[f32], mut out: impl FnMut(f32)) {
        for frame in frames.chunks_exact(self.channels) {
            let x = frame.iter().sum::<f32>() / self.channels as f32;
            while self.pos <= 1.0 {
                out(self.prev + (x - self.prev) * self.pos as f32);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = x;
        }
    }
}

/// Open the default microphone. Its samples arrive in the returned queue, mono, at
/// `sample_rate`.
pub fn start(sample_rate: u32) -> Result<(AudioInput, rtrb::Consumer<f32>), InputError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(InputError::NoDevice)?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "Microphone".into());
    // Ask for the engine's rate when the device can do it, so nothing is resampled.
    let supported = device
        .supported_input_configs()
        .ok()
        .and_then(|mut configs| configs.find_map(|c| c.try_with_sample_rate(sample_rate)))
        .map(Ok)
        .unwrap_or_else(|| device.default_input_config())
        .map_err(|e| InputError::Build(e.to_string()))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let (tx, rx) = rtrb::RingBuffer::new((sample_rate * QUEUE_SECONDS) as usize);
    let convert = ToMono::new(config.channels as usize, config.sample_rate, sample_rate);
    let failed = Arc::new(AtomicBool::new(false));

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, tx, convert, failed.clone()),
        SampleFormat::I16 => build::<i16>(&device, config, tx, convert, failed.clone()),
        SampleFormat::U16 => build::<u16>(&device, config, tx, convert, failed.clone()),
        SampleFormat::I32 => build::<i32>(&device, config, tx, convert, failed.clone()),
        SampleFormat::F64 => build::<f64>(&device, config, tx, convert, failed.clone()),
        other => {
            return Err(InputError::Build(format!(
                "unsupported sample format {other}"
            )));
        }
    }?;
    stream
        .play()
        .map_err(|e| InputError::Build(e.to_string()))?;
    Ok((
        AudioInput {
            _stream: stream,
            name,
            failed,
        },
        rx,
    ))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut tx: rtrb::Producer<f32>,
    mut convert: ToMono,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, InputError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    // Converted samples, reused every callback (sized for the largest we expect).
    let mut frames = Vec::with_capacity(16_384);
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                frames.clear();
                frames.extend(data.iter().map(|s| s.to_sample::<f32>()));
                convert.push(&frames, |s| {
                    // The engine isn't keeping up (or is gone): drop rather than block.
                    let _ = tx.push(s);
                });
            },
            move |err: cpal::Error| {
                eprintln!("microphone error: {err}");
                if matches!(err.kind(), cpal::ErrorKind::DeviceNotAvailable) {
                    failed.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| InputError::Build(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut ToMono, frames: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        c.push(frames, |s| out.push(s));
        out
    }

    #[test]
    fn same_rate_mono_passes_through() {
        let mut c = ToMono::new(1, 48_000, 48_000);
        assert_eq!(run(&mut c, &[0.1, 0.2, 0.3]), vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn stereo_is_averaged() {
        let mut c = ToMono::new(2, 48_000, 48_000);
        assert_eq!(run(&mut c, &[1.0, 0.0, 0.5, 0.5]), vec![0.5, 0.5]);
    }

    #[test]
    fn upsampling_interpolates_between_frames() {
        let mut c = ToMono::new(1, 24_000, 48_000);
        let out = run(&mut c, &[1.0, 1.0, 0.0, 0.0]);
        // Two outputs per input, bar the very first frame, which has no neighbour yet.
        assert_eq!(out, vec![1.0, 1.0, 1.0, 0.5, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn conversion_keeps_its_place_across_callbacks() {
        let mut whole = ToMono::new(1, 44_100, 48_000);
        let mut split = ToMono::new(1, 44_100, 48_000);
        let input: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.01).sin()).collect();
        let a = run(&mut whole, &input);
        let mut b = run(&mut split, &input[..1000]);
        b.extend(run(&mut split, &input[1000..]));
        assert_eq!(a, b);
        assert!((a.len() as i64 - 4800).abs() <= 1, "{} samples", a.len());
    }
}
