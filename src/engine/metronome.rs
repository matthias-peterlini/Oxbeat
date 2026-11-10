//! The metronome's click: a short, bright blip, higher on the first beat of a bar.

use std::f32::consts::TAU;

const BEAT_HZ: f32 = 1000.0;
const BAR_HZ: f32 = 1600.0;
const BEAT_LEVEL: f32 = 0.35;
const BAR_LEVEL: f32 = 0.5;
/// Time for the click to fade to about a third.
const DECAY_SECONDS: f32 = 0.012;
/// Below this the click is over.
const SILENT: f32 = 1e-4;

pub struct Click {
    sr: f32,
    phase: f32,
    step: f32,
    level: f32,
    decay: f32,
}

impl Click {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(1) as f32;
        Self {
            sr,
            phase: 0.0,
            step: 0.0,
            level: 0.0,
            decay: (-1.0 / (DECAY_SECONDS * sr)).exp(),
        }
    }

    /// Start a click; `accent` marks the first beat of a bar.
    pub fn trigger(&mut self, accent: bool) {
        self.phase = 0.0;
        self.step = if accent { BAR_HZ } else { BEAT_HZ } / self.sr;
        self.level = if accent { BAR_LEVEL } else { BEAT_LEVEL };
    }

    pub fn is_silent(&self) -> bool {
        self.level < SILENT
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.level < SILENT {
            self.level = 0.0;
            return 0.0;
        }
        let out = (TAU * self.phase).sin() * self.level;
        self.phase += self.step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.level *= self.decay;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_is_short_and_bounded() {
        let mut c = Click::new(48_000);
        assert!(c.is_silent());
        assert_eq!(c.next(), 0.0);
        c.trigger(true);
        let out: Vec<f32> = (0..9600).map(|_| c.next()).collect();
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.2 && peak <= BAR_LEVEL);
        // Gone within a fifth of a second.
        assert!(out[7200..].iter().all(|x| *x == 0.0));
        assert!(c.is_silent());
    }

    #[test]
    fn bar_click_is_louder_than_beat_click() {
        let level = |accent| {
            let mut c = Click::new(48_000);
            c.trigger(accent);
            (0..480).map(|_| c.next().abs()).fold(0.0f32, f32::max)
        };
        assert!(level(true) > level(false));
    }
}
