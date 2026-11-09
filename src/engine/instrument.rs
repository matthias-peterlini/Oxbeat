//! What a track plays through. Built off the audio thread when it owns heap data
//! (samples, SoundFonts); built-in synths and kits are heap-free.

use super::drums::{DrumKit, KITS};
use super::sampler::Sampler;
use super::synth::{PRESETS, Synth};

// Variants stay inline (not boxed) so the engine can build built-ins without allocating.
#[allow(clippy::large_enum_variant)]
pub enum Instrument {
    /// Not loaded yet (or failed to load): silent.
    None,
    Synth(Synth),
    Drums(DrumKit),
    Sampler(Sampler),
    SoundFont(Box<SoundFontInstrument>),
}

/// One SoundFont preset on one MIDI channel (channel 10 for drum kits).
pub struct SoundFontInstrument {
    pub synth: rustysynth::Synthesizer,
    pub channel: i32,
}

impl Instrument {
    /// A built-in sound by name, if it is one. Never allocates.
    pub fn builtin(sound: &str, sample_rate: u32) -> Option<Instrument> {
        let name = sound.strip_prefix("builtin:").unwrap_or(sound);
        if PRESETS.contains(&name) {
            Some(Instrument::Synth(Synth::new(name, sample_rate)))
        } else if KITS.contains(&name) {
            Some(Instrument::Drums(DrumKit::new(name, sample_rate)))
        } else {
            None
        }
    }

    /// Dropping this value frees no memory, so it may be dropped on the audio thread.
    pub fn is_heap_free(&self) -> bool {
        matches!(
            self,
            Instrument::None | Instrument::Synth(_) | Instrument::Drums(_)
        )
    }

    pub fn note_on(&mut self, pitch: u8, velocity: u8) {
        match self {
            Instrument::None => {}
            Instrument::Synth(s) => s.note_on(pitch, velocity),
            Instrument::Drums(d) => d.note_on(pitch, velocity),
            Instrument::Sampler(s) => s.note_on(pitch, velocity),
            Instrument::SoundFont(f) => f.synth.note_on(f.channel, pitch as i32, velocity as i32),
        }
    }

    pub fn note_off(&mut self, pitch: u8) {
        match self {
            Instrument::None => {}
            Instrument::Synth(s) => s.note_off(pitch),
            Instrument::Drums(d) => d.note_off(pitch),
            Instrument::Sampler(s) => s.note_off(pitch),
            Instrument::SoundFont(f) => f.synth.note_off(f.channel, pitch as i32),
        }
    }

    pub fn all_notes_off(&mut self) {
        match self {
            Instrument::None => {}
            Instrument::Synth(s) => s.all_notes_off(),
            Instrument::Drums(d) => d.all_notes_off(),
            Instrument::Sampler(s) => s.all_notes_off(),
            Instrument::SoundFont(f) => f.synth.note_off_all(false),
        }
    }

    /// Render into `left`/`right`, which hold silence on entry.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        match self {
            Instrument::None => {}
            Instrument::Synth(s) => {
                s.render(left);
                right.copy_from_slice(left);
            }
            Instrument::Drums(d) => d.render(left, right),
            Instrument::Sampler(s) => s.render(left, right),
            Instrument::SoundFont(f) => f.synth.render(left, right),
        }
    }
}

/// Stable hash of a sound id, comparable on the audio thread without allocating.
pub fn sound_hash(sound: &str) -> u64 {
    // FNV-1a
    sound.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}
