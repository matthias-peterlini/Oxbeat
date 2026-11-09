//! Pitch correction: listen for the note being sung, and shift it onto the nearest
//! note of the scale.
//!
//! Detection is YIN (de Cheveigné & Kawahara, 2002) on a downsampled copy. Shifting
//! reads the sound back from a short delay line at a slightly different speed; when
//! the read point drifts too far it jumps by exactly one period of the note, which
//! lands on the same point of the waveform and so is inaudible.

use crate::project::effects::Effect;

/// Lowest and highest notes we try to follow (covers bass to soprano and most
/// instruments someone would play into a microphone).
const MIN_HZ: f32 = 70.0;
const MAX_HZ: f32 = 1000.0;
/// Rate the detector works at; plenty for fundamentals below 1 kHz.
const DETECT_RATE: f32 = 24_000.0;
/// Length of sound compared against itself, in seconds.
const WINDOW_SECONDS: f32 = 0.022;
/// A new estimate every this many detector samples.
const HOP: usize = 256;
/// YIN's "this dip is a real period" threshold.
const YIN_THRESHOLD: f32 = 0.15;
/// Quieter than this, there is no note to follow.
const SILENCE_RMS: f32 = 0.003;
/// Major scale steps from its root.
const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Furthest a note is ever moved, in semitones.
const MAX_SHIFT: f32 = 2.0;

/// Period (in samples of `x`) of the note in `x`, if there is one. `x` must hold at
/// least `window + max_lag` samples; `scratch` at least `max_lag + 1`.
pub fn detect_period(
    x: &[f32],
    window: usize,
    min_lag: usize,
    max_lag: usize,
    scratch: &mut [f32],
) -> Option<f32> {
    let energy: f32 = x[..window].iter().map(|s| s * s).sum();
    if (energy / window as f32).sqrt() < SILENCE_RMS {
        return None;
    }
    // Difference of the sound against itself shifted by each lag, normalised by
    // the running mean so that small lags don't win by default.
    let d = &mut scratch[..=max_lag];
    d[0] = 1.0;
    let mut running = 0.0f32;
    for lag in 1..=max_lag {
        let mut sum = 0.0f32;
        for j in 0..window {
            let diff = x[j] - x[j + lag];
            sum += diff * diff;
        }
        running += sum;
        d[lag] = if running > 0.0 {
            sum * lag as f32 / running
        } else {
            1.0
        };
    }
    let mut lag = min_lag.max(2);
    while lag < max_lag {
        if d[lag] < YIN_THRESHOLD {
            // Walk down to the bottom of this dip.
            while lag + 1 < max_lag && d[lag + 1] < d[lag] {
                lag += 1;
            }
            // Refine between samples with a parabola through the three points.
            let (a, b, c) = (d[lag - 1], d[lag], d[lag + 1]);
            let denom = a - 2.0 * b + c;
            let shift = if denom.abs() > 1e-12 {
                (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            return Some(lag as f32 + shift);
        }
        lag += 1;
    }
    None
}

/// Which of the 12 pitch classes a scale choice allows (bit 0 = C).
fn scale_mask(choice: usize) -> u16 {
    if choice == 0 || choice > 12 {
        return 0x0fff;
    }
    let root = (choice - 1) as u8;
    MAJOR
        .iter()
        .fold(0, |mask, step| mask | 1 << ((root + step) % 12))
}

/// Semitones from `midi` (fractional note number) to the nearest allowed note.
fn correction(midi: f32, mask: u16) -> f32 {
    let nearest = midi.round() as i32;
    let mut best = 0.0f32;
    let mut best_dist = f32::MAX;
    for n in nearest - 2..=nearest + 2 {
        if mask & (1 << n.rem_euclid(12)) == 0 {
            continue;
        }
        let dist = (n as f32 - midi).abs();
        if dist < best_dist {
            best_dist = dist;
            best = n as f32 - midi;
        }
    }
    best.clamp(-MAX_SHIFT, MAX_SHIFT)
}

pub struct PitchCorrect {
    sr: f32,
    strength: f32,
    /// Per-sample smoothing of the shift (1.0 = instant).
    glide: f32,
    mask: u16,

    // Detector.
    decimate: usize,
    dec_sum: f32,
    dec_count: usize,
    window: usize,
    min_lag: usize,
    max_lag: usize,
    /// The last `window + max_lag` detector samples, as a ring.
    recent: Vec<f32>,
    recent_pos: usize,
    /// The same, unrolled oldest-first for the detector.
    linear: Vec<f32>,
    scratch: Vec<f32>,
    until_detect: usize,

    // Shifter.
    lines: [Vec<f32>; 2],
    write: usize,
    /// Period of the current note in samples (kept through unvoiced sounds).
    period: f32,
    /// Playback speed we are heading for, and the smoothed one in use.
    target_ratio: f32,
    ratio: f32,
    /// How far behind the write point we read.
    delay: f32,
    /// A jump in progress: the delay we are fading to and how far along (0..1).
    jump: Option<(f32, f32)>,
}

impl PitchCorrect {
    pub fn new(sr: f32) -> Self {
        let decimate = ((sr / DETECT_RATE).round() as usize).max(1);
        let rate = sr / decimate as f32;
        let window = (rate * WINDOW_SECONDS) as usize;
        let max_lag = (rate / MIN_HZ) as usize;
        let min_lag = ((rate / MAX_HZ) as usize).max(2);
        let period = sr / 200.0;
        // Room for the furthest read: a bit over two periods of the lowest note.
        let len = ((sr / MIN_HZ) * 4.0) as usize;
        let len = len.next_power_of_two();
        Self {
            sr,
            strength: 1.0,
            glide: 1.0,
            mask: 0x0fff,
            decimate,
            dec_sum: 0.0,
            dec_count: 0,
            window,
            min_lag,
            max_lag,
            recent: vec![0.0; window + max_lag],
            recent_pos: 0,
            linear: vec![0.0; window + max_lag],
            scratch: vec![0.0; max_lag + 1],
            until_detect: HOP,
            lines: [vec![0.0; len], vec![0.0; len]],
            write: 0,
            period,
            target_ratio: 1.0,
            ratio: 1.0,
            delay: period * 1.25 + 2.0,
            jump: None,
        }
    }

    pub fn set(&mut self, e: &Effect) {
        self.strength = e.get(0);
        let ms = e.get(1);
        self.glide = if ms <= 0.5 {
            1.0
        } else {
            1.0 - (-1.0 / (ms * 0.001 * self.sr)).exp()
        };
        self.mask = scale_mask(e.get(2).round() as usize);
    }

    fn detect(&mut self) {
        let n = self.recent.len();
        let (newer, older) = self.recent.split_at(self.recent_pos);
        self.linear[..older.len()].copy_from_slice(older);
        self.linear[older.len()..n].copy_from_slice(newer);
        let found = detect_period(
            &self.linear,
            self.window,
            self.min_lag,
            self.max_lag,
            &mut self.scratch,
        );
        match found {
            Some(lag) => {
                self.period = lag * self.decimate as f32;
                let hz = self.sr / self.period;
                let midi = 69.0 + 12.0 * (hz / 440.0).log2();
                let shift = correction(midi, self.mask) * self.strength;
                self.target_ratio = 2f32.powf(shift / 12.0);
            }
            None => self.target_ratio = 1.0,
        }
    }

    #[inline]
    fn read(&self, ch: usize, delay: f32) -> f32 {
        let line = &self.lines[ch];
        let mask = line.len() - 1;
        let whole = delay as usize;
        let frac = delay - whole as f32;
        let a = line[(self.write + line.len() - whole) & mask];
        let b = line[(self.write + line.len() - whole - 1) & mask];
        a + (b - a) * frac
    }

    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mask = self.lines[0].len() - 1;
        let longest = (mask - 2) as f32;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            // Feed the detector.
            self.dec_sum += (*xl + *xr) * 0.5;
            self.dec_count += 1;
            if self.dec_count == self.decimate {
                self.recent[self.recent_pos] = self.dec_sum / self.decimate as f32;
                self.recent_pos = (self.recent_pos + 1) % self.recent.len();
                self.dec_sum = 0.0;
                self.dec_count = 0;
                self.until_detect -= 1;
                if self.until_detect == 0 {
                    self.until_detect = HOP;
                    self.detect();
                }
            }

            self.write = (self.write + 1) & mask;
            self.lines[0][self.write] = *xl;
            self.lines[1][self.write] = *xr;

            // Reading faster than we write (ratio > 1) raises the pitch and eats
            // into the delay; slower lowers it and falls behind.
            self.ratio += (self.target_ratio - self.ratio) * self.glide;
            let drift = 1.0 - self.ratio;
            self.delay = (self.delay + drift).clamp(1.0, longest);
            let period = self.period.max(16.0);
            match &mut self.jump {
                Some((to, progress)) => {
                    *to = (*to + drift).clamp(1.0, longest);
                    *progress += 2.0 / period;
                    let (to, t) = (*to, progress.min(1.0));
                    *xl = self.read(0, self.delay) * (1.0 - t) + self.read(0, to) * t;
                    *xr = self.read(1, self.delay) * (1.0 - t) + self.read(1, to) * t;
                    if t >= 1.0 {
                        self.delay = to;
                        self.jump = None;
                    }
                }
                None => {
                    *xl = self.read(0, self.delay);
                    *xr = self.read(1, self.delay);
                    if self.delay < period * 0.25 + 2.0 {
                        self.jump = Some((self.delay + period, 0.0));
                    } else if self.delay > period * 2.25 + 2.0 {
                        self.jump = Some((self.delay - period, 0.0));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fx::testing::peak;
    use crate::project::effects::EffectKind;

    const SR: f32 = 48_000.0;

    /// A voice-like tone: a fundamental with a few harmonics.
    fn voice(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                (1..=5)
                    .map(|h| (std::f32::consts::TAU * hz * h as f32 * t).sin() / h as f32)
                    .sum::<f32>()
                    * 0.3
            })
            .collect()
    }

    /// Pitch of the end of `x`, measured at full rate.
    fn pitch_of(x: &[f32]) -> Option<f32> {
        let (window, max_lag) = (2048, 700);
        let tail = &x[x.len() - window - max_lag..];
        let mut scratch = vec![0.0; max_lag + 1];
        detect_period(tail, window, 40, max_lag, &mut scratch).map(|p| SR / p)
    }

    #[test]
    fn detector_finds_the_fundamental() {
        for hz in [82.4, 220.0, 329.6, 440.0, 880.0] {
            let found = pitch_of(&voice(hz, 9600)).unwrap();
            assert!((found - hz).abs() < hz * 0.004, "{hz} Hz read as {found}");
        }
    }

    #[test]
    fn detector_ignores_silence_and_noise() {
        assert_eq!(pitch_of(&vec![0.0; 9600]), None);
        let mut seed = 22_222u32;
        let noise: Vec<f32> = (0..9600)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect();
        assert_eq!(pitch_of(&noise), None);
    }

    #[test]
    fn scales_allow_the_right_notes() {
        assert_eq!(scale_mask(0), 0x0fff);
        // C major: C D E F G A B.
        assert_eq!(scale_mask(1), 0b1010_1011_0101);
        // G major has F sharp, not F.
        assert!(scale_mask(8) & (1 << 6) != 0 && scale_mask(8) & (1 << 5) == 0);
    }

    #[test]
    fn correction_snaps_to_the_scale() {
        assert!((correction(60.3, 0x0fff) + 0.3).abs() < 1e-5);
        // C sharp isn't in C major: 61.2 goes up to D rather than down to C sharp.
        assert!((correction(61.2, scale_mask(1)) - 0.8).abs() < 1e-5);
    }

    fn corrected(hz: f32, effect: &Effect) -> Vec<f32> {
        let mut p = PitchCorrect::new(SR);
        p.set(effect);
        let mut l = voice(hz, 48_000);
        let mut r = l.clone();
        // Odd block sizes, like a real audio callback.
        let mut at = 0;
        for n in [441, 512, 37].iter().cycle() {
            let end = (at + n).min(l.len());
            p.process(&mut l[at..end], &mut r[at..end]);
            at = end;
            if at == l.len() {
                break;
            }
        }
        assert_eq!(l, r);
        l
    }

    #[test]
    fn flat_note_is_pulled_into_tune() {
        // 225 Hz sits between A3 (220) and B flat 3 (233.1), nearer A.
        let fast = Effect::with(EffectKind::PitchCorrect, &[(1, 0.0)]);
        let out = corrected(225.0, &fast);
        let hz = pitch_of(&out).unwrap();
        assert!((hz - 220.0).abs() < 1.0, "came out at {hz} Hz");
        assert!(peak(&out) < 1.0);
    }

    #[test]
    fn sharp_note_is_pulled_down_within_the_scale() {
        // 450 Hz is a sharp A4; in C major it should land on A (440).
        let e = Effect::with(EffectKind::PitchCorrect, &[(1, 0.0), (2, 1.0)]);
        let hz = pitch_of(&corrected(450.0, &e)).unwrap();
        assert!((hz - 440.0).abs() < 2.0, "came out at {hz} Hz");
    }

    #[test]
    fn zero_strength_and_in_tune_notes_keep_their_pitch() {
        let off = Effect::with(EffectKind::PitchCorrect, &[(0, 0.0)]);
        let hz = pitch_of(&corrected(225.0, &off)).unwrap();
        assert!((hz - 225.0).abs() < 0.5, "{hz}");
        let hz = pitch_of(&corrected(220.0, &Effect::new(EffectKind::PitchCorrect))).unwrap();
        assert!((hz - 220.0).abs() < 0.5, "{hz}");
    }

    #[test]
    fn jumps_do_not_click() {
        let fast = Effect::with(EffectKind::PitchCorrect, &[(1, 0.0)]);
        let out = corrected(225.0, &fast);
        // The steepest slope of the tone itself is about 0.1 per sample; a click
        // would be a far bigger step.
        let steepest = out[24_000..]
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(steepest < 0.2, "step of {steepest}");
    }

    #[test]
    fn silence_in_silence_out() {
        let mut p = PitchCorrect::new(SR);
        p.set(&Effect::new(EffectKind::PitchCorrect));
        let (mut l, mut r) = (vec![0.0; 4096], vec![0.0; 4096]);
        p.process(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| *x == 0.0));
    }
}
