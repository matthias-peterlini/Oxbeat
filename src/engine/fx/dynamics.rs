//! Loudness control: compressor, noise gate, de-esser and the master limiter.

use super::tone::Biquad;
use crate::project::effects::Effect;

/// One-pole smoothing coefficient for a time constant in milliseconds.
fn coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sr)).exp()
    }
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

const KNEE_DB: f32 = 6.0;

/// Turns loud parts down (both channels together, so the stereo image holds).
pub struct Compressor {
    sr: f32,
    threshold: f32,
    /// 1/ratio - 1: how much of the overshoot is removed (negative).
    slope: f32,
    attack: f32,
    release: f32,
    makeup_db: f32,
    /// Current gain reduction in dB (<= 0).
    reduction: f32,
}

impl Compressor {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            threshold: 0.0,
            slope: 0.0,
            attack: 0.0,
            release: 0.0,
            makeup_db: 0.0,
            reduction: 0.0,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        self.threshold = e.get(0);
        self.slope = 1.0 / e.get(1).max(1.0) - 1.0;
        self.attack = coef(e.get(2), self.sr);
        self.release = coef(e.get(3), self.sr);
        self.makeup_db = e.get(4);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let level = 20.0 * (xl.abs().max(xr.abs()) + 1e-9).log10();
            let over = level - self.threshold;
            let target = if over <= -KNEE_DB / 2.0 {
                0.0
            } else if over < KNEE_DB / 2.0 {
                let x = over + KNEE_DB / 2.0;
                self.slope * x * x / (2.0 * KNEE_DB)
            } else {
                self.slope * over
            };
            let c = if target < self.reduction {
                self.attack
            } else {
                self.release
            };
            self.reduction = target + (self.reduction - target) * c;
            let gain = db_to_gain(self.reduction + self.makeup_db);
            *xl *= gain;
            *xr *= gain;
        }
    }
}

const GATE_ATTACK_MS: f32 = 1.0;
const GATE_HOLD_MS: f32 = 40.0;
const GATE_DETECTOR_MS: f32 = 10.0;
/// The gate closes this far below where it opens, so it doesn't chatter.
const GATE_HYSTERESIS: f32 = 0.5;

/// Mutes the track while it is quieter than the threshold.
pub struct Gate {
    sr: f32,
    threshold: f32,
    release: f32,
    attack: f32,
    detector: f32,
    env: f32,
    gain: f32,
    hold: u32,
    hold_samples: u32,
    open: bool,
}

impl Gate {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            threshold: 0.0,
            release: 0.0,
            attack: coef(GATE_ATTACK_MS, sr),
            detector: coef(GATE_DETECTOR_MS, sr),
            env: 0.0,
            gain: 0.0,
            hold: 0,
            hold_samples: (GATE_HOLD_MS * 0.001 * sr) as u32,
            open: false,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        self.threshold = db_to_gain(e.get(0));
        self.release = coef(e.get(1), self.sr);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let peak = xl.abs().max(xr.abs());
            self.env = peak.max(self.env * self.detector);
            if self.env >= self.threshold {
                self.open = true;
                self.hold = self.hold_samples;
            } else if self.env < self.threshold * GATE_HYSTERESIS {
                if self.hold > 0 {
                    self.hold -= 1;
                } else {
                    self.open = false;
                }
            }
            let (target, c) = if self.open {
                (1.0, self.attack)
            } else {
                (0.0, self.release)
            };
            self.gain = target + (self.gain - target) * c;
            if self.gain < 1e-6 {
                self.gain = 0.0;
            }
            *xl *= self.gain;
            *xr *= self.gain;
        }
    }
}

const DEESS_ATTACK_MS: f32 = 1.0;
const DEESS_RELEASE_MS: f32 = 60.0;
/// The band that gets turned down starts this far below the frequency knob, so the
/// whole hiss is caught rather than only its top.
const DEESS_SPLIT: f32 = 0.6;
/// Q of a Linkwitz-Riley crossover: its two bands add back up to the original level.
const CROSSOVER_Q: f32 = 0.5;

/// Turns the top of the spectrum down only while it is loud (an "S" or "T").
pub struct DeEsser {
    sr: f32,
    /// Per channel: the low band and the high band of a crossover.
    low: [Biquad; 2],
    high: [Biquad; 2],
    threshold: f32,
    active: bool,
    attack: f32,
    release: f32,
    env: f32,
}

impl DeEsser {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            low: [Biquad::flat(); 2],
            high: [Biquad::flat(); 2],
            threshold: 1.0,
            active: false,
            attack: coef(DEESS_ATTACK_MS, sr),
            release: coef(DEESS_RELEASE_MS, sr),
            env: 0.0,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        let split = e.get(0) * DEESS_SPLIT;
        for (low, high) in self.low.iter_mut().zip(&mut self.high) {
            low.lowpass(self.sr, split, CROSSOVER_Q);
            high.highpass(self.sr, split, CROSSOVER_Q);
        }
        let amount = e.get(1);
        self.active = amount > 0.0;
        self.threshold = db_to_gain(-10.0 - 30.0 * amount);
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            // The high band of this crossover comes out upside down; flip it back.
            let (ll, lr) = (self.low[0].run(*xl), self.low[1].run(*xr));
            let (hl, hr) = (-self.high[0].run(*xl), -self.high[1].run(*xr));
            let peak = hl.abs().max(hr.abs());
            let c = if peak > self.env {
                self.attack
            } else {
                self.release
            };
            self.env = peak + (self.env - peak) * c;
            let keep = if self.active && self.env > self.threshold {
                // Remove most of what sticks out above the threshold.
                (self.threshold / self.env).powf(0.75)
            } else {
                1.0
            };
            *xl = ll + hl * keep;
            *xr = lr + hr * keep;
        }
    }
}

/// Loudest the master output may get (just under full scale).
pub const CEILING: f32 = 0.98;
const LIMITER_RELEASE_MS: f32 = 80.0;

/// Keeps the final mix from clipping: gain drops the instant a peak would pass the
/// ceiling and recovers smoothly afterwards.
pub struct Limiter {
    release: f32,
    env: f32,
}

impl Limiter {
    pub fn new(sr: f32) -> Self {
        Self {
            release: coef(LIMITER_RELEASE_MS, sr),
            env: 0.0,
        }
    }

    /// Limit one frame; returns the gain it applied.
    #[inline]
    pub fn run(&mut self, frame: &mut [f32]) -> f32 {
        let peak = frame.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        self.env = peak.max(self.env * self.release);
        if self.env < 1e-9 {
            self.env = 0.0;
        }
        if self.env <= CEILING {
            return 1.0;
        }
        let gain = CEILING / self.env;
        for x in frame {
            *x *= gain;
        }
        gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fx::testing::{peak, rms, sine};
    use crate::project::effects::EffectKind;

    const SR: f32 = 48_000.0;

    fn scaled(v: &[f32], g: f32) -> Vec<f32> {
        v.iter().map(|x| x * g).collect()
    }

    #[test]
    fn compressor_leaves_quiet_sound_alone_and_turns_loud_sound_down() {
        let mut c = Compressor::new(SR);
        c.set(&Effect::with(
            EffectKind::Compressor,
            &[(0, -20.0), (1, 4.0), (2, 1.0)],
        ));
        let mut quiet = scaled(&sine(440.0, SR, 24_000), 0.01);
        let before = rms(&quiet);
        c.process(&mut quiet, &mut vec![0.0; 24_000]);
        assert!((rms(&quiet) / before - 1.0).abs() < 0.01);

        // 0 dB in, 20 dB over a 4:1 threshold: 15 dB should go.
        let mut loud = sine(440.0, SR, 48_000);
        let mut right = loud.clone();
        c.process(&mut loud, &mut right);
        let db = 20.0 * peak(&loud[24_000..]).log10();
        assert!((db + 15.0).abs() < 1.5, "{db} dB");
        assert_eq!(loud, right);
    }

    #[test]
    fn gate_mutes_hiss_and_passes_the_voice() {
        let mut g = Gate::new(SR);
        g.set(&Effect::with(EffectKind::Gate, &[(0, -40.0), (1, 50.0)]));
        let mut hiss = scaled(&sine(3000.0, SR, 24_000), 0.001);
        g.process(&mut hiss, &mut vec![0.0; 24_000]);
        assert_eq!(peak(&hiss), 0.0);
        let mut voice = scaled(&sine(300.0, SR, 24_000), 0.5);
        g.process(&mut voice, &mut vec![0.0; 24_000]);
        assert!((peak(&voice[12_000..]) - 0.5).abs() < 0.01);
        // ...and closes again once it stops.
        let mut tail = scaled(&sine(3000.0, SR, 48_000), 0.001);
        g.process(&mut tail, &mut vec![0.0; 48_000]);
        assert_eq!(peak(&tail[40_000..]), 0.0);
    }

    #[test]
    fn deesser_tames_sibilance_but_not_the_body() {
        let mut d = DeEsser::new(SR);
        d.set(&Effect::with(EffectKind::DeEsser, &[(1, 1.0)]));
        let mut ess = scaled(&sine(8000.0, SR, 24_000), 0.5);
        d.process(&mut ess, &mut vec![0.0; 24_000]);
        assert!(peak(&ess[12_000..]) < 0.2, "{}", peak(&ess[12_000..]));
        let mut body = scaled(&sine(300.0, SR, 24_000), 0.5);
        d.process(&mut body, &mut vec![0.0; 24_000]);
        assert!(peak(&body[12_000..]) > 0.48);
    }

    #[test]
    fn limiter_never_passes_the_ceiling() {
        let mut lim = Limiter::new(SR);
        let hot = scaled(&sine(220.0, SR, 9600), 3.0);
        let mut out = Vec::new();
        for x in hot {
            let mut frame = [x, -x * 0.5];
            lim.run(&mut frame);
            out.extend(frame);
        }
        assert!(peak(&out) <= CEILING + 1e-6);
        assert!(peak(&out) > 0.9);
    }

    #[test]
    fn limiter_leaves_normal_levels_untouched() {
        let mut lim = Limiter::new(SR);
        for x in scaled(&sine(220.0, SR, 4800), 0.9) {
            let mut frame = [x, x];
            assert_eq!(lim.run(&mut frame), 1.0);
            assert_eq!(frame[0], x);
        }
    }
}
