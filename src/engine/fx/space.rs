//! Effects built on delay lines: reverb, echo and chorus.

use std::f32::consts::TAU;

use crate::project::effects::Effect;

#[inline]
fn undenormal(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

/// A fixed-size delay line read at a fractional distance behind the write point.
struct Line {
    buf: Vec<f32>,
    pos: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(4).next_power_of_two()],
            pos: 0,
        }
    }

    #[inline]
    fn write(&mut self, x: f32) {
        self.pos = (self.pos + 1) & (self.buf.len() - 1);
        self.buf[self.pos] = x;
    }

    /// The sample written `delay` samples ago (linear interpolation).
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let mask = self.buf.len() - 1;
        let delay = delay.clamp(0.0, (mask - 1) as f32);
        let whole = delay as usize;
        let frac = delay - whole as f32;
        let a = self.buf[(self.pos + self.buf.len() - whole) & mask];
        let b = self.buf[(self.pos + self.buf.len() - whole - 1) & mask];
        a + (b - a) * frac
    }
}

// ---- Reverb (Freeverb, by Jezar at Dreampoint; public domain) ----

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// Extra delay on the right channel, which is what makes the tail wide.
const STEREO_SPREAD: usize = 23;
const TUNED_FOR: f32 = 44_100.0;
const INPUT_GAIN: f32 = 0.015;
const WET_GAIN: f32 = 3.0;

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            pos: 0,
            store: 0.0,
        }
    }

    #[inline]
    fn run(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = undenormal(out * (1.0 - damp) + self.store * damp);
        self.buf[self.pos] = x + self.store * feedback;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.pos];
        self.buf[self.pos] = undenormal(x + delayed * 0.5);
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        delayed - x
    }
}

pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    feedback: f32,
    damp: f32,
    mix: f32,
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        let scale = |n: usize| ((n as f32) * sr / TUNED_FOR).round() as usize;
        let side = |spread: usize| {
            (
                COMBS
                    .iter()
                    .map(|n| Comb::new(scale(n + spread)))
                    .collect::<Vec<_>>(),
                ALLPASSES
                    .iter()
                    .map(|n| Allpass::new(scale(n + spread)))
                    .collect::<Vec<_>>(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(STEREO_SPREAD);
        Self {
            combs: [cl, cr],
            allpasses: [al, ar],
            feedback: 0.84,
            damp: 0.2,
            mix: 0.25,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        self.feedback = 0.7 + 0.28 * e.get(0);
        self.damp = 0.4 * e.get(1);
        self.mix = e.get(2);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let input = (*xl + *xr) * INPUT_GAIN;
            let mut wet = [0.0f32; 2];
            let sides = self.combs.iter_mut().zip(&mut self.allpasses);
            for (out, (combs, allpasses)) in wet.iter_mut().zip(sides) {
                let mut sum = 0.0;
                for c in combs {
                    sum += c.run(input, self.feedback, self.damp);
                }
                for a in allpasses {
                    sum = a.run(sum);
                }
                *out = sum * WET_GAIN;
            }
            *xl = *xl * (1.0 - self.mix) + wet[0] * self.mix;
            *xr = *xr * (1.0 - self.mix) + wet[1] * self.mix;
        }
    }
}

// ---- Delay ----

const MAX_DELAY_SECONDS: f32 = 1.6;
/// How quickly the echo time follows the knob (per sample); gliding avoids clicks.
const TIME_GLIDE: f32 = 0.0004;

/// Repeating echoes.
pub struct Delay {
    sr: f32,
    lines: [Line; 2],
    /// Delay in samples: where the knob is, and where we have glided to.
    target: f32,
    current: f32,
    feedback: f32,
    mix: f32,
}

impl Delay {
    pub fn new(sr: f32) -> Self {
        let len = (MAX_DELAY_SECONDS * sr) as usize;
        Self {
            sr,
            lines: [Line::new(len), Line::new(len)],
            target: 0.0,
            current: 0.0,
            feedback: 0.0,
            mix: 0.0,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        self.target = e.get(0) * 0.001 * self.sr;
        if self.current == 0.0 {
            self.current = self.target;
        }
        self.feedback = e.get(1);
        self.mix = e.get(2);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            self.current += (self.target - self.current) * TIME_GLIDE;
            for (line, x) in self.lines.iter_mut().zip([xl, xr]) {
                let echo = line.read(self.current - 1.0);
                line.write(undenormal(*x + echo * self.feedback));
                *x += echo * self.mix;
            }
        }
    }
}

// ---- Chorus ----

const CHORUS_CENTER_MS: f32 = 14.0;
const CHORUS_SWING_MS: f32 = 6.0;

/// A copy of the sound whose delay drifts slowly, like a second player.
pub struct Chorus {
    sr: f32,
    lines: [Line; 2],
    phase: f32,
    step: f32,
    depth: f32,
    mix: f32,
}

impl Chorus {
    pub fn new(sr: f32) -> Self {
        let len = ((CHORUS_CENTER_MS + CHORUS_SWING_MS + 2.0) * 0.001 * sr) as usize;
        Self {
            sr,
            lines: [Line::new(len), Line::new(len)],
            phase: 0.0,
            step: 0.0,
            depth: 0.0,
            mix: 0.0,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        self.step = e.get(0) / self.sr;
        self.depth = e.get(1);
        self.mix = e.get(2);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let center = CHORUS_CENTER_MS * 0.001 * self.sr;
        let swing = CHORUS_SWING_MS * 0.001 * self.sr * self.depth;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            self.phase += self.step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            // The right channel runs a quarter turn ahead, which widens the sound.
            for (i, (line, x)) in self.lines.iter_mut().zip([xl, xr]).enumerate() {
                line.write(*x);
                let lfo = (TAU * (self.phase + 0.25 * i as f32)).sin();
                let wet = line.read(center + swing * lfo);
                *x = *x * (1.0 - self.mix * 0.5) + wet * self.mix * 0.5;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fx::testing::{peak, sine};
    use crate::project::effects::EffectKind;

    const SR: f32 = 48_000.0;

    fn impulse(n: usize) -> (Vec<f32>, Vec<f32>) {
        let mut v = vec![0.0; n];
        v[0] = 1.0;
        (v.clone(), v)
    }

    #[test]
    fn reverb_rings_after_the_sound_stops_then_dies_away() {
        let mut rv = Reverb::new(SR);
        rv.set(&Effect::with(EffectKind::Reverb, &[(0, 0.5), (2, 1.0)]));
        let (mut l, mut r) = impulse(48_000 * 6);
        rv.process(&mut l, &mut r);
        assert!(peak(&l[2400..24_000]) > 0.001, "no tail");
        assert!(peak(&l[48_000 * 5..]) < 1e-4, "tail never ends");
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 1.0));
        // Left and right differ: the tail is stereo.
        assert!(l[4800..9600] != r[4800..9600]);
    }

    #[test]
    fn reverb_fully_dry_is_the_input() {
        let mut rv = Reverb::new(SR);
        rv.set(&Effect::with(EffectKind::Reverb, &[(2, 0.0)]));
        let mut l = sine(440.0, SR, 4800);
        let mut r = l.clone();
        let before = l.clone();
        rv.process(&mut l, &mut r);
        assert_eq!(l, before);
    }

    #[test]
    fn delay_echoes_at_the_set_time_and_fades() {
        let mut d = Delay::new(SR);
        d.set(&Effect::with(
            EffectKind::Delay,
            &[(0, 100.0), (1, 0.5), (2, 1.0)],
        ));
        let (mut l, mut r) = impulse(24_000);
        d.process(&mut l, &mut r);
        assert_eq!(l[0], 1.0);
        assert!((l[4800] - 1.0).abs() < 1e-3, "first echo {}", l[4800]);
        assert!((l[9600] - 0.5).abs() < 1e-3, "second echo {}", l[9600]);
        assert!(peak(&l[1..4700]) < 1e-6);
    }

    #[test]
    fn chorus_keeps_level_sane_and_moves_the_sound() {
        let mut c = Chorus::new(SR);
        c.set(&Effect::new(EffectKind::Chorus));
        let mut l = sine(440.0, SR, 48_000);
        let mut r = l.clone();
        let before = l.clone();
        c.process(&mut l, &mut r);
        assert!(peak(&l) <= 1.01);
        assert!(l != before);
        assert!(l != r);
    }

    #[test]
    fn silence_in_silence_out() {
        let e = |k| Effect::new(k);
        let (mut l, mut r) = (vec![0.0; 2048], vec![0.0; 2048]);
        let mut rv = Reverb::new(SR);
        rv.set(&e(EffectKind::Reverb));
        rv.process(&mut l, &mut r);
        let mut d = Delay::new(SR);
        d.set(&e(EffectKind::Delay));
        d.process(&mut l, &mut r);
        let mut c = Chorus::new(SR);
        c.set(&e(EffectKind::Chorus));
        c.process(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| *x == 0.0));
    }
}
