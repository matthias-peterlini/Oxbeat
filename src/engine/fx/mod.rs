//! Audio effects and the per-track chain that runs them.
//!
//! A chain owns delay lines and other buffers, so it is built off the audio thread,
//! handed to the engine in a command, and handed back to be freed. Turning knobs only
//! needs `configure`, which never allocates.

pub mod dynamics;
pub mod pitch;
pub mod space;
pub mod tone;

use crate::project::effects::{Effect, EffectKind};

enum Unit {
    Gate(dynamics::Gate),
    PitchCorrect(Box<pitch::PitchCorrect>),
    DeEsser(dynamics::DeEsser),
    Eq(tone::Eq),
    Compressor(dynamics::Compressor),
    Distortion(tone::Distortion),
    Chorus(space::Chorus),
    Delay(space::Delay),
    Reverb(Box<space::Reverb>),
}

impl Unit {
    fn new(kind: EffectKind, sr: f32) -> Unit {
        match kind {
            EffectKind::Gate => Unit::Gate(dynamics::Gate::new(sr)),
            EffectKind::PitchCorrect => Unit::PitchCorrect(Box::new(pitch::PitchCorrect::new(sr))),
            EffectKind::DeEsser => Unit::DeEsser(dynamics::DeEsser::new(sr)),
            EffectKind::Eq => Unit::Eq(tone::Eq::new(sr)),
            EffectKind::Compressor => Unit::Compressor(dynamics::Compressor::new(sr)),
            EffectKind::Distortion => Unit::Distortion(tone::Distortion::new(sr)),
            EffectKind::Chorus => Unit::Chorus(space::Chorus::new(sr)),
            EffectKind::Delay => Unit::Delay(space::Delay::new(sr)),
            EffectKind::Reverb => Unit::Reverb(Box::new(space::Reverb::new(sr))),
        }
    }

    fn set(&mut self, e: &Effect) {
        match self {
            Unit::Gate(u) => u.set(e),
            Unit::PitchCorrect(u) => u.set(e),
            Unit::DeEsser(u) => u.set(e),
            Unit::Eq(u) => u.set(e),
            Unit::Compressor(u) => u.set(e),
            Unit::Distortion(u) => u.set(e),
            Unit::Chorus(u) => u.set(e),
            Unit::Delay(u) => u.set(e),
            Unit::Reverb(u) => u.set(e),
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        match self {
            Unit::Gate(u) => u.process(l, r),
            Unit::PitchCorrect(u) => u.process(l, r),
            Unit::DeEsser(u) => u.process(l, r),
            Unit::Eq(u) => u.process(l, r),
            Unit::Compressor(u) => u.process(l, r),
            Unit::Distortion(u) => u.process(l, r),
            Unit::Chorus(u) => u.process(l, r),
            Unit::Delay(u) => u.process(l, r),
            Unit::Reverb(u) => u.process(l, r),
        }
    }
}

struct Slot {
    kind: EffectKind,
    on: bool,
    unit: Unit,
}

/// The effects of one track, ready to run.
pub struct Chain {
    slots: Vec<Slot>,
}

impl Chain {
    /// Build the DSP for `effects`. Allocates: call off the audio thread.
    pub fn build(effects: &[Effect], sample_rate: u32) -> Chain {
        let sr = sample_rate.max(1) as f32;
        let mut chain = Chain {
            slots: effects
                .iter()
                .map(|e| Slot {
                    kind: e.kind,
                    on: e.on,
                    unit: Unit::new(e.kind, sr),
                })
                .collect(),
        };
        chain.configure(effects);
        chain
    }

    /// The kinds of effect in the chain, in order.
    pub fn kinds(effects: &[Effect]) -> Vec<EffectKind> {
        effects.iter().map(|e| e.kind).collect()
    }

    /// True when this chain was built for effects of these kinds in this order.
    pub fn matches(&self, effects: &[Effect]) -> bool {
        self.slots.len() == effects.len()
            && self
                .slots
                .iter()
                .zip(effects)
                .all(|(s, e)| s.kind == e.kind)
    }

    /// Take over the knob positions of `effects`. Does nothing unless the chain was
    /// built for them (a rebuilt chain is then on its way). Never allocates.
    pub fn configure(&mut self, effects: &[Effect]) {
        if !self.matches(effects) {
            return;
        }
        for (slot, e) in self.slots.iter_mut().zip(effects) {
            slot.on = e.on;
            slot.unit.set(e);
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for slot in &mut self.slots {
            if slot.on {
                slot.unit.process(l, r);
            }
        }
    }
}

#[cfg(test)]
pub mod testing {
    pub fn sine(hz: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * hz * i as f32 / sr).sin())
            .collect()
    }

    pub fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    pub fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{peak, sine};
    use super::*;

    #[test]
    fn empty_chain_passes_sound_through() {
        let mut c = Chain::build(&[], 48_000);
        let mut l = sine(440.0, 48_000.0, 512);
        let mut r = l.clone();
        let before = l.clone();
        c.process(&mut l, &mut r);
        assert_eq!(l, before);
    }

    #[test]
    fn switched_off_effects_are_skipped() {
        let mut e = Effect::with(EffectKind::Distortion, &[(0, 1.0)]);
        e.on = false;
        let mut c = Chain::build(&[e], 48_000);
        let mut l = sine(440.0, 48_000.0, 512);
        let before = l.clone();
        c.process(&mut l, &mut vec![0.0; 512]);
        assert_eq!(l, before);
    }

    #[test]
    fn configure_follows_knobs_but_not_a_different_chain() {
        let quiet = Effect::with(EffectKind::Eq, &[(0, 0.0)]);
        let mut c = Chain::build(std::slice::from_ref(&quiet), 48_000);
        assert!(c.matches(std::slice::from_ref(&quiet)));
        let other = [Effect::new(EffectKind::Reverb)];
        assert!(!c.matches(&other));
        c.configure(&other);
        assert!(c.matches(&[quiet]));

        c.configure(&[Effect::with(EffectKind::Eq, &[(1, 15.0)])]);
        let mut l = sine(1000.0, 48_000.0, 9600);
        c.process(&mut l, &mut vec![0.0; 9600]);
        assert!(peak(&l[4800..]) > 4.0);
    }

    #[test]
    fn every_effect_survives_loud_input_at_any_setting() {
        for kind in EffectKind::ALL {
            for extreme in [0.0f32, 1.0] {
                let mut e = Effect::new(kind);
                for (i, spec) in kind.params().iter().enumerate() {
                    e.set(i, spec.min + (spec.max - spec.min) * extreme);
                }
                for sr in [22_050, 44_100, 96_000] {
                    let mut c = Chain::build(std::slice::from_ref(&e), sr);
                    let mut l = sine(330.0, sr as f32, 8192);
                    let mut r = l.clone();
                    for (a, b) in l.chunks_mut(300).zip(r.chunks_mut(300)) {
                        c.process(a, b);
                    }
                    assert!(
                        l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 40.0),
                        "{kind:?} at {extreme} / {sr} Hz"
                    );
                }
            }
        }
    }
}
