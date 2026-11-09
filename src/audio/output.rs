//! Opens the default output device and drives the `Engine` from its callback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, SupportedBufferSize};

use crate::engine::Engine;
use crate::engine::commands::{EngineCommand, EngineEvent};

/// Requested buffer: 256 frames ≈ 5 ms at 48 kHz.
const PREFERRED_FRAMES: u32 = 256;
/// Largest callback we pre-size the conversion buffer for (frames × channels grows as needed
/// only when the device asks for more, which happens at most once).
const MAX_SCRATCH: usize = 16_384;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("No audio device found")]
    NoDevice,
    #[error("Could not start audio: {0}")]
    Build(String),
}

pub struct AudioOutput {
    _stream: cpal::Stream,
    failed: Arc<AtomicBool>,
    sample_rate: u32,
}

impl AudioOutput {
    /// True once the device reported a fatal error (e.g. unplugged).
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

pub fn start(
    cmd_rx: rtrb::Consumer<EngineCommand>,
    evt_tx: rtrb::Producer<EngineEvent>,
) -> Result<AudioOutput, AudioError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
    let supported = device
        .default_output_config()
        .map_err(|e| AudioError::Build(e.to_string()))?;
    let format = supported.sample_format();
    let buffer_size = match supported.buffer_size() {
        SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&PREFERRED_FRAMES) => {
            cpal::BufferSize::Fixed(PREFERRED_FRAMES)
        }
        _ => cpal::BufferSize::Default,
    };
    let mut config: cpal::StreamConfig = supported.into();
    config.buffer_size = buffer_size;
    let sample_rate = config.sample_rate;
    let engine = Engine::new(sample_rate, cmd_rx, evt_tx);
    let failed = Arc::new(AtomicBool::new(false));

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, engine, failed.clone()),
        SampleFormat::I16 => build::<i16>(&device, config, engine, failed.clone()),
        SampleFormat::U16 => build::<u16>(&device, config, engine, failed.clone()),
        SampleFormat::I32 => build::<i32>(&device, config, engine, failed.clone()),
        SampleFormat::F64 => build::<f64>(&device, config, engine, failed.clone()),
        other => {
            return Err(AudioError::Build(format!(
                "unsupported sample format {other}"
            )));
        }
    }?;
    stream
        .play()
        .map_err(|e| AudioError::Build(e.to_string()))?;
    Ok(AudioOutput {
        _stream: stream,
        failed,
        sample_rate,
    })
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut engine: Engine,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut scratch = vec![0.0f32; MAX_SCRATCH];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                // Process in pieces that fit the preallocated buffer.
                let per = (MAX_SCRATCH / channels) * channels;
                for out in data.chunks_mut(per) {
                    let buf = &mut scratch[..out.len()];
                    engine.process(buf, channels);
                    for (o, s) in out.iter_mut().zip(buf.iter()) {
                        *o = T::from_sample(*s);
                    }
                }
            },
            move |err: cpal::Error| {
                eprintln!("audio stream error: {err}");
                // Glitches (xruns etc.) are survivable; a vanished device is not.
                if matches!(err.kind(), cpal::ErrorKind::DeviceNotAvailable) {
                    failed.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| AudioError::Build(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_for_no_device() {
        assert_eq!(AudioError::NoDevice.to_string(), "No audio device found");
    }
}
