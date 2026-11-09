//! Tone shaping: the biquad filter, a three-band equalizer and distortion.

use std::f32::consts::{PI, SQRT_2};

use crate::project::effects::Effect;

/// A second-order filter (RBJ cookbook coefficients, transposed direct form II).
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// A filter that passes sound through unchanged.
    pub fn flat() -> Self {
        Self {
            b0: 1.0,
            ..Self::default()
        }
    }

    fn set(&mut self, b: [f32; 3], a: [f32; 3]) {
        let inv = 1.0 / a[0];
        self.b0 = b[0] * inv;
        self.b1 = b[1] * inv;
        self.b2 = b[2] * inv;
        self.a1 = a[1] * inv;
        self.a2 = a[2] * inv;
    }

    fn angle(sr: f32, freq: f32) -> (f32, f32) {
        let w = 2.0 * PI * (freq / sr).clamp(0.0005, 0.49);
        (w.cos(), w.sin())
    }

    pub fn low_shelf(&mut self, sr: f32, freq: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let (cos, sin) = Self::angle(sr, freq);
        let k = 2.0 * a.sqrt() * (sin / 2.0 * SQRT_2);
        self.set(
            [
                a * ((a + 1.0) - (a - 1.0) * cos + k),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - k),
            ],
            [
                (a + 1.0) + (a - 1.0) * cos + k,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - k,
            ],
        );
    }

    pub fn high_shelf(&mut self, sr: f32, freq: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let (cos, sin) = Self::angle(sr, freq);
        let k = 2.0 * a.sqrt() * (sin / 2.0 * SQRT_2);
        self.set(
            [
                a * ((a + 1.0) + (a - 1.0) * cos + k),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - k),
            ],
            [
                (a + 1.0) - (a - 1.0) * cos + k,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - k,
            ],
        );
    }

    pub fn peak(&mut self, sr: f32, freq: f32, q: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let (cos, sin) = Self::angle(sr, freq);
        let alpha = sin / (2.0 * q);
        self.set(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        );
    }

    pub fn lowpass(&mut self, sr: f32, freq: f32, q: f32) {
        let (cos, sin) = Self::angle(sr, freq);
        let alpha = sin / (2.0 * q);
        self.set(
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        );
    }

    pub fn highpass(&mut self, sr: f32, freq: f32, q: f32) {
        let (cos, sin) = Self::angle(sr, freq);
        let alpha = sin / (2.0 * q);
        self.set(
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        );
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        // Keep the state out of the (slow) denormal range once the sound has died away.
        if self.z1.abs() < 1e-20 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-20 {
            self.z2 = 0.0;
        }
        y
    }
}

const LOW_SHELF_HZ: f32 = 120.0;
const HIGH_SHELF_HZ: f32 = 8000.0;
const MID_Q: f32 = 0.9;

/// Low shelf, sweepable mid bell, high shelf.
pub struct Eq {
    sr: f32,
    /// Three bands per channel.
    bands: [[Biquad; 3]; 2],
}

impl Eq {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            bands: [[Biquad::flat(); 3]; 2],
        }
    }

    pub fn set(&mut self, e: &Effect) {
        for ch in &mut self.bands {
            ch[0].low_shelf(self.sr, LOW_SHELF_HZ, e.get(0));
            ch[1].peak(self.sr, e.get(2), MID_Q, e.get(1));
            ch[2].high_shelf(self.sr, HIGH_SHELF_HZ, e.get(3));
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (ch, buf) in self.bands.iter_mut().zip([l, r]) {
            for x in buf.iter_mut() {
                let mut y = *x;
                for band in ch.iter_mut() {
                    y = band.run(y);
                }
                *x = y;
            }
        }
    }
}

/// Most gain the drive knob adds before the clipper.
const MAX_DRIVE_DB: f32 = 36.0;

/// A soft clipper with a tone control after it.
pub struct Distortion {
    sr: f32,
    gain: f32,
    level: f32,
    mix: f32,
    tone: [Biquad; 2],
}

impl Distortion {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            gain: 1.0,
            level: 1.0,
            mix: 1.0,
            tone: [Biquad::flat(); 2],
        }
    }

    pub fn set(&mut self, e: &Effect) {
        let drive = e.get(0);
        self.gain = 10f32.powf(drive * MAX_DRIVE_DB / 20.0);
        // Driving harder is louder; pull the level back so the knob changes the
        // character more than the volume.
        self.level = 1.0 / (1.0 + 1.5 * drive);
        self.mix = e.get(2);
        let cutoff = 800.0 * 2f32.powf(e.get(1) * 4.3);
        for t in &mut self.tone {
            t.lowpass(self.sr, cutoff, 0.707);
        }
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (tone, buf) in self.tone.iter_mut().zip([l, r]) {
            for x in buf.iter_mut() {
                let wet = tone.run((*x * self.gain).tanh()) * self.level;
                *x += (wet - *x) * self.mix;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fx::testing::{rms, sine};
    use crate::project::effects::EffectKind;

    #[test]
    fn flat_eq_changes_nothing() {
        let mut eq = Eq::new(48_000.0);
        eq.set(&Effect::new(EffectKind::Eq));
        let mut l = sine(440.0, 48_000.0, 4800);
        let mut r = l.clone();
        let before = l.clone();
        eq.process(&mut l, &mut r);
        for (a, b) in l.iter().zip(&before) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn low_shelf_boosts_bass_and_leaves_treble() {
        let mut eq = Eq::new(48_000.0);
        eq.set(&Effect::with(EffectKind::Eq, &[(0, 12.0)]));
        let mut bass = sine(50.0, 48_000.0, 48_000);
        let mut treble = sine(5000.0, 48_000.0, 48_000);
        let (b0, t0) = (rms(&bass[24_000..]), rms(&treble[24_000..]));
        eq.process(&mut bass, &mut vec![0.0; 48_000]);
        eq.process(&mut treble, &mut vec![0.0; 48_000]);
        let gain_db = 20.0 * (rms(&bass[24_000..]) / b0).log10();
        assert!((gain_db - 12.0).abs() < 1.5, "{gain_db} dB");
        assert!((rms(&treble[24_000..]) / t0 - 1.0).abs() < 0.05);
    }

    #[test]
    fn distortion_stays_bounded_and_adds_harmonics() {
        let mut d = Distortion::new(48_000.0);
        d.set(&Effect::with(EffectKind::Distortion, &[(0, 1.0), (1, 1.0)]));
        let mut l = sine(200.0, 48_000.0, 9600);
        let mut r = l.clone();
        let clean = l.clone();
        d.process(&mut l, &mut r);
        assert!(l.iter().all(|x| x.abs() <= 1.0));
        // A clipped sine is squarer: more of its energy sits at full level.
        let crest = |v: &[f32]| v.iter().fold(0.0f32, |m, x| m.max(x.abs())) / rms(v);
        assert!(crest(&l[4800..]) < crest(&clean[4800..]) - 0.1);
    }

    #[test]
    fn silence_stays_silent() {
        let mut d = Distortion::new(48_000.0);
        d.set(&Effect::new(EffectKind::Distortion));
        let mut eq = Eq::new(48_000.0);
        eq.set(&Effect::with(
            EffectKind::Eq,
            &[(0, 9.0), (1, -6.0), (3, 9.0)],
        ));
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        d.process(&mut l, &mut r);
        eq.process(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| *x == 0.0));
    }
}
