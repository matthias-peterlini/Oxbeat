//! Built-in synthesized drum kits, mapped to General MIDI drum notes.
//! Everything is preallocated; safe to use on the audio thread.

use std::f32::consts::TAU;

pub const KITS: [&str; 2] = ["Studio Kit", "808 Kit"];
const VOICES: usize = 24;
const PEAK: f32 = 0.5;

/// The pads shown in the app: (GM note, label).
pub const PADS: [(u8, &str); 12] = [
    (36, "Kick"),
    (38, "Snare"),
    (42, "Hi-Hat"),
    (46, "Open Hat"),
    (39, "Clap"),
    (37, "Rim"),
    (45, "Low Tom"),
    (47, "Mid Tom"),
    (50, "High Tom"),
    (49, "Crash"),
    (51, "Ride"),
    (56, "Cowbell"),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Piece {
    Kick,
    Snare,
    Clap,
    Rim,
    ClosedHat,
    OpenHat,
    Tom(f32),
    Crash,
    Ride,
    Cowbell,
    Shaker,
}

fn piece(note: u8) -> Option<Piece> {
    Some(match note {
        35 | 36 => Piece::Kick,
        37 => Piece::Rim,
        38 | 40 => Piece::Snare,
        39 => Piece::Clap,
        42 | 44 => Piece::ClosedHat,
        46 => Piece::OpenHat,
        41 => Piece::Tom(70.0),
        43 => Piece::Tom(85.0),
        45 => Piece::Tom(100.0),
        47 => Piece::Tom(125.0),
        48 => Piece::Tom(150.0),
        50 => Piece::Tom(180.0),
        49 | 52 | 55 | 57 => Piece::Crash,
        51 | 53 | 59 => Piece::Ride,
        56 => Piece::Cowbell,
        54 | 69 | 70 | 82 => Piece::Shaker,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    piece: Piece,
    note: u8,
    active: bool,
    t: u32,
    gain: f32,
    phase: f32,
    phase2: f32,
    /// One-pole filter states.
    hp: f32,
    lp: f32,
    prev: f32,
    /// Fade-out gain when choked (open hat by closed hat).
    choke: f32,
    age: u64,
}

impl Voice {
    const IDLE: Voice = Voice {
        piece: Piece::Kick,
        note: 0,
        active: false,
        t: 0,
        gain: 0.0,
        phase: 0.0,
        phase2: 0.0,
        hp: 0.0,
        lp: 0.0,
        prev: 0.0,
        choke: 1.0,
        age: 0,
    };
}

pub struct DrumKit {
    sr: f32,
    is_808: bool,
    voices: [Voice; VOICES],
    noise: u32,
    counter: u64,
}

impl DrumKit {
    pub fn new(kit: &str, sample_rate: u32) -> DrumKit {
        DrumKit {
            sr: sample_rate as f32,
            is_808: kit == "808 Kit",
            voices: [Voice::IDLE; VOICES],
            noise: 0x1234_5678,
            counter: 0,
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let Some(p) = piece(note) else { return };
        if matches!(p, Piece::ClosedHat) {
            for v in &mut self.voices {
                if v.active && v.piece == Piece::OpenHat {
                    v.choke = 0.999;
                }
            }
        }
        self.counter += 1;
        let slot = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap());
        let mut v = Voice::IDLE;
        v.piece = p;
        v.note = note;
        v.active = true;
        v.gain = (velocity.min(127) as f32 / 127.0).powf(1.5) * PEAK;
        v.age = self.counter;
        self.voices[slot] = v;
    }

    /// Drums ring out on their own; note-off does nothing.
    pub fn note_off(&mut self, _note: u8) {}

    pub fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.choke = 0.995;
            }
        }
    }

    #[cfg(test)]
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    fn white(noise: &mut u32) -> f32 {
        // xorshift32
        let mut x = *noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *noise = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Stereo; writes into zeroed (or partially filled) buffers by adding.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sr;
        let is_808 = self.is_808;
        for v in &mut self.voices {
            if !v.active {
                continue;
            }
            // Per-piece constant pan.
            let pan: f32 = match v.piece {
                Piece::ClosedHat | Piece::OpenHat | Piece::Shaker => 0.3,
                Piece::Ride => -0.25,
                Piece::Crash => -0.3,
                Piece::Tom(f) => ((f - 120.0) / 120.0).clamp(-0.4, 0.4),
                Piece::Cowbell => 0.2,
                _ => 0.0,
            };
            let (gl, gr) = ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
            for i in 0..left.len() {
                let t = v.t as f32 / sr;
                let n = Self::white(&mut self.noise);
                let (s, length) = match v.piece {
                    Piece::Kick => {
                        let (start, end, decay) = if is_808 {
                            (110.0, 45.0, 0.9)
                        } else {
                            (160.0, 52.0, 0.35)
                        };
                        let f = end + (start - end) * (-t * 28.0).exp();
                        v.phase = (v.phase + f / sr).fract();
                        let body = (v.phase * TAU).sin() * (-t / decay * 3.0).exp();
                        let click = if is_808 {
                            0.0
                        } else {
                            n * (-t * 400.0).exp() * 0.3
                        };
                        (body + click, decay * 1.5)
                    }
                    Piece::Snare => {
                        v.phase = (v.phase + 185.0 / sr).fract();
                        let tone = (v.phase * TAU).sin() * (-t * 30.0).exp() * 0.6;
                        // High-passed noise.
                        v.hp = 0.6 * (v.hp + n - v.prev);
                        v.prev = n;
                        let decay = if is_808 { 12.0 } else { 16.0 };
                        (tone + v.hp * (-t * decay).exp() * 0.9, 0.45)
                    }
                    Piece::Clap => {
                        let burst = [0.0, 0.011, 0.022, 0.031]
                            .iter()
                            .map(|o| {
                                let d = t - o;
                                if d >= 0.0 { (-d * 180.0).exp() } else { 0.0 }
                            })
                            .fold(0.0f32, f32::max);
                        let tail = if t > 0.031 {
                            (-(t - 0.031) * 14.0).exp() * 0.5
                        } else {
                            0.0
                        };
                        v.lp += 0.35 * (n - v.lp);
                        let bp = n - v.lp;
                        (bp * (burst + tail), 0.4)
                    }
                    Piece::Rim => {
                        v.phase = (v.phase + 1700.0 / sr).fract();
                        let s = ((v.phase * TAU).sin() * 0.7 + n * 0.3) * (-t * 90.0).exp();
                        (s, 0.08)
                    }
                    Piece::ClosedHat | Piece::OpenHat | Piece::Shaker => {
                        v.hp = 0.92 * (v.hp + n - v.prev);
                        v.prev = n;
                        let decay = match v.piece {
                            Piece::ClosedHat => 60.0,
                            Piece::Shaker => 30.0,
                            _ => 6.0,
                        };
                        (v.hp * (-t * decay).exp() * 0.6, 7.0 / decay)
                    }
                    Piece::Tom(f0) => {
                        let f = f0 * (1.0 + 0.5 * (-t * 20.0).exp());
                        v.phase = (v.phase + f / sr).fract();
                        let s =
                            (v.phase * TAU).sin() * (-t * 7.0).exp() + n * (-t * 60.0).exp() * 0.15;
                        (s, 0.7)
                    }
                    Piece::Crash | Piece::Ride => {
                        // Metallic: a few inharmonic squares plus noise.
                        let base = if v.piece == Piece::Ride { 420.0 } else { 340.0 };
                        v.phase = (v.phase + base * 1.47 / sr).fract();
                        v.phase2 = (v.phase2 + base * 2.13 / sr).fract();
                        let metal = (if v.phase < 0.5 { 1.0 } else { -1.0 })
                            * (if v.phase2 < 0.5 { 1.0 } else { -1.0 });
                        let mix = metal * 0.3 + n * 0.7;
                        v.hp = 0.85 * (v.hp + mix - v.prev);
                        v.prev = mix;
                        let decay = if v.piece == Piece::Ride { 2.2 } else { 1.6 };
                        (v.hp * (-t * decay).exp() * 0.55, 5.0 / decay)
                    }
                    Piece::Cowbell => {
                        v.phase = (v.phase + 545.0 / sr).fract();
                        v.phase2 = (v.phase2 + 815.0 / sr).fract();
                        let sq = |p: f32| if p < 0.5 { 1.0 } else { -1.0 };
                        let s = (sq(v.phase) + sq(v.phase2)) * 0.3 * (-t * 9.0).exp();
                        v.lp += 0.3 * (s - v.lp);
                        (v.lp, 0.6)
                    }
                };
                let out = s * v.gain * v.choke.min(1.0);
                if v.choke < 1.0 {
                    v.choke *= 0.999;
                }
                left[i] += out * gl;
                right[i] += out * gr;
                v.t += 1;
                if t > length || v.choke < 0.001 {
                    *v = Voice::IDLE;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(k: &mut DrumKit, n: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        k.render(&mut l, &mut r);
        (l, r)
    }

    fn peak(b: &[f32]) -> f32 {
        b.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    #[test]
    fn silent_without_hits() {
        let mut k = DrumKit::new("Studio Kit", 48_000);
        let (l, r) = render(&mut k, 1024);
        assert!(l.iter().chain(&r).all(|x| *x == 0.0));
    }

    #[test]
    fn every_pad_sounds_and_stops() {
        for kit in KITS {
            for (note, name) in PADS {
                let mut k = DrumKit::new(kit, 48_000);
                k.note_on(note, 127);
                let (l, r) = render(&mut k, 48_000 * 6);
                let p = peak(&l).max(peak(&r));
                assert!(p > 0.01, "{kit} {name} is silent");
                assert!(p <= 1.0, "{kit} {name} too loud: {p}");
                assert_eq!(k.active_voices(), 0, "{kit} {name} never ends");
            }
        }
    }

    #[test]
    fn unknown_note_is_ignored() {
        let mut k = DrumKit::new("Studio Kit", 48_000);
        k.note_on(10, 100);
        assert_eq!(k.active_voices(), 0);
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let mut k = DrumKit::new("Studio Kit", 48_000);
        k.note_on(46, 127);
        render(&mut k, 1000);
        k.note_on(42, 127);
        render(&mut k, 48_000 / 4);
        assert_eq!(k.active_voices(), 0);
    }
}
