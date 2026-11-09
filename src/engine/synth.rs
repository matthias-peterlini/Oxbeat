//! Built-in polyphonic synth. Everything is preallocated; safe to use on the audio thread.

use std::f32::consts::TAU;

pub const PRESETS: [&str; 4] = ["Keys", "Bass", "Lead", "Pad"];
const VOICES: usize = 16;
const VOICE_PEAK: f32 = 0.25;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
}

#[derive(Clone, Copy, Debug)]
struct Patch {
    osc: [(Wave, f32 /*cents detune*/, f32 /*mix*/); 2],
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    cutoff_hz: f32,
}

fn patch(preset: &str) -> Patch {
    match preset {
        "Bass" => Patch {
            osc: [(Wave::Saw, 0.0, 1.0), (Wave::Sine, -1200.0, 0.0)],
            attack: 0.002,
            decay: 0.25,
            sustain: 0.7,
            release: 0.08,
            cutoff_hz: 800.0,
        },
        "Lead" => Patch {
            osc: [(Wave::Square, 0.0, 1.0), (Wave::Sine, 0.0, 0.0)],
            attack: 0.005,
            decay: 0.1,
            sustain: 0.8,
            release: 0.12,
            cutoff_hz: 4000.0,
        },
        "Pad" => Patch {
            osc: [(Wave::Saw, 7.0, 0.5), (Wave::Saw, -7.0, 0.5)],
            attack: 0.4,
            decay: 0.5,
            sustain: 0.8,
            release: 0.8,
            cutoff_hz: 2500.0,
        },
        // "Keys" and anything unknown.
        _ => Patch {
            osc: [(Wave::Triangle, 0.0, 0.6), (Wave::Sine, 1200.0, 0.4)],
            attack: 0.005,
            decay: 0.3,
            sustain: 0.4,
            release: 0.25,
            cutoff_hz: 6000.0,
        },
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    pitch: u8,
    gain: f32,
    phase: [f32; 2],
    inc: [f32; 2],
    stage: Stage,
    level: f32,
    release_step: f32,
    lp: f32,
    age: u64,
}

impl Voice {
    const IDLE: Voice = Voice {
        pitch: 0,
        gain: 0.0,
        phase: [0.0; 2],
        inc: [0.0; 2],
        stage: Stage::Idle,
        level: 0.0,
        release_step: 0.0,
        lp: 0.0,
        age: 0,
    };
}

fn osc(wave: Wave, phase: f32) -> f32 {
    match wave {
        Wave::Sine => (phase * TAU).sin(),
        Wave::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
        Wave::Saw => 2.0 * phase - 1.0,
        Wave::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

pub struct Synth {
    patch: Patch,
    sample_rate: f32,
    voices: [Voice; VOICES],
    lp_coef: f32,
    counter: u64,
}

impl Synth {
    pub fn new(preset: &str, sample_rate: u32) -> Synth {
        let patch = patch(preset);
        let sr = sample_rate as f32;
        Synth {
            patch,
            sample_rate: sr,
            voices: [Voice::IDLE; VOICES],
            lp_coef: 1.0 - (-TAU * patch.cutoff_hz / sr).exp(),
            counter: 0,
        }
    }

    pub fn note_on(&mut self, pitch: u8, velocity: u8) {
        self.counter += 1;
        let slot = self
            .voices
            .iter()
            .position(|v| v.stage == Stage::Idle)
            .unwrap_or_else(|| {
                // Steal the oldest voice.
                (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap()
            });
        let freq = 440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0);
        let mut v = Voice::IDLE;
        v.pitch = pitch;
        v.gain = velocity.min(127) as f32 / 127.0 * VOICE_PEAK;
        for (i, (_, cents, _)) in self.patch.osc.iter().enumerate() {
            v.inc[i] = freq * 2f32.powf(cents / 1200.0) / self.sample_rate;
        }
        v.stage = Stage::Attack;
        v.age = self.counter;
        self.voices[slot] = v;
    }

    pub fn note_off(&mut self, pitch: u8) {
        let release_samples = (self.patch.release * self.sample_rate).max(1.0);
        for v in &mut self.voices {
            if v.pitch == pitch && v.stage != Stage::Idle && v.stage != Stage::Release {
                v.stage = Stage::Release;
                v.release_step = v.level / release_samples;
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for p in 0..=127u8 {
            self.note_off(p);
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.stage != Stage::Idle)
            .count()
    }

    /// Mono; adds into `out`.
    pub fn render(&mut self, out: &mut [f32]) {
        let sr = self.sample_rate;
        let p = self.patch;
        let attack_step = 1.0 / (p.attack * sr).max(1.0);
        let decay_step = (1.0 - p.sustain) / (p.decay * sr).max(1.0);
        for v in &mut self.voices {
            if v.stage == Stage::Idle {
                continue;
            }
            for sample in out.iter_mut() {
                match v.stage {
                    Stage::Attack => {
                        v.level += attack_step;
                        if v.level >= 1.0 {
                            v.level = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        v.level -= decay_step;
                        if v.level <= p.sustain {
                            v.level = p.sustain;
                            v.stage = Stage::Sustain;
                        }
                    }
                    Stage::Sustain => {}
                    Stage::Release => {
                        v.level -= v.release_step;
                        if v.level <= 0.0 {
                            v.level = 0.0;
                            v.stage = Stage::Idle;
                        }
                    }
                    Stage::Idle => break,
                }
                let mut s = 0.0;
                for i in 0..2 {
                    let (wave, _, mix) = p.osc[i];
                    if mix > 0.0 {
                        s += osc(wave, v.phase[i]) * mix;
                    }
                    v.phase[i] = (v.phase[i] + v.inc[i]).fract();
                }
                v.lp += self.lp_coef * (s - v.lp);
                *sample += v.lp * v.level * v.gain;
            }
            if v.stage == Stage::Idle {
                *v = Voice::IDLE;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(b: &[f32]) -> f32 {
        b.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    #[test]
    fn silent_without_notes() {
        let mut s = Synth::new("Keys", 48_000);
        let mut b = [0.0f32; 512];
        s.render(&mut b);
        assert!(b.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn note_makes_sound() {
        for preset in PRESETS {
            let mut s = Synth::new(preset, 48_000);
            s.note_on(60, 100);
            let mut b = vec![0.0f32; 48_000];
            s.render(&mut b);
            assert!(peak(&b) > 0.01, "{preset} is silent");
            assert!(peak(&b) <= VOICE_PEAK + 1e-3, "{preset} too loud");
        }
    }

    #[test]
    fn release_goes_silent() {
        let mut s = Synth::new("Keys", 48_000);
        s.note_on(60, 100);
        let mut b = vec![0.0f32; 4_800];
        s.render(&mut b);
        s.note_off(60);
        let mut tail = vec![0.0f32; 48_000];
        s.render(&mut tail);
        assert_eq!(s.active_voices(), 0);
        let mut last = [0.0f32; 512];
        s.render(&mut last);
        assert!(last.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn steals_beyond_16() {
        let mut s = Synth::new("Pad", 48_000);
        for p in 40..60 {
            s.note_on(p, 100);
        }
        assert_eq!(s.active_voices(), 16);
    }

    #[test]
    fn all_notes_off_releases() {
        let mut s = Synth::new("Pad", 48_000);
        for p in [60, 64, 67] {
            s.note_on(p, 100);
        }
        let mut b = vec![0.0f32; 4_800];
        s.render(&mut b);
        s.all_notes_off();
        let mut tail = vec![0.0f32; 48_000];
        s.render(&mut tail);
        assert_eq!(s.active_voices(), 0);
    }
}
