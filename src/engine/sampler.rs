//! Multi-sample instrument (SFZ, single audio files). Sample data is shared and
//! read-only; voices are preallocated, so note handling and rendering never allocate.

use std::sync::Arc;

const VOICES: usize = 48;
/// Shortest release, so notes never click off.
const MIN_RELEASE: f32 = 0.006;
/// Fast fade for voices cut by `off_by` groups and voice stealing.
const CHOKE_SECONDS: f32 = 0.01;

/// Decoded audio: interleaved stereo frames at `rate`.
#[derive(Debug, Default)]
pub struct SampleData {
    pub rate: u32,
    pub frames: Vec<f32>,
}

impl SampleData {
    pub fn len(&self) -> usize {
        self.frames.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum LoopMode {
    #[default]
    NoLoop,
    OneShot,
    Continuous,
    Sustain,
}

#[derive(Clone, Debug)]
pub struct Region {
    pub sample: Arc<SampleData>,
    pub lokey: u8,
    pub hikey: u8,
    pub lovel: u8,
    pub hivel: u8,
    pub key_center: f32,
    /// 0..=1 how much pitch follows the key (SFZ pitch_keytrack / 100).
    pub keytrack: f32,
    /// Fine tune in semitones (tune + transpose).
    pub tune: f32,
    /// Linear gain.
    pub gain: f32,
    /// -1..=1
    pub pan: f32,
    /// 0..=1 how much velocity affects loudness.
    pub veltrack: f32,
    pub loop_mode: LoopMode,
    pub loop_start: usize,
    pub loop_end: usize,
    pub offset: usize,
    pub end: usize,
    pub attack: f32,
    pub decay: f32,
    /// 0..=1
    pub sustain: f32,
    pub release: f32,
    pub group: u32,
    pub off_by: u32,
    /// Random range (0..=1) for round robins via lorand/hirand.
    pub lorand: f32,
    pub hirand: f32,
    /// Round robin: play when (count % seq_length) + 1 == seq_position.
    pub seq_length: u32,
    pub seq_position: u32,
}

impl Region {
    /// A region playing `sample` across the whole keyboard, unpitched at `key_center`.
    pub fn whole(sample: Arc<SampleData>, key_center: u8) -> Region {
        let len = sample.len();
        Region {
            sample,
            lokey: 0,
            hikey: 127,
            lovel: 0,
            hivel: 127,
            key_center: key_center as f32,
            keytrack: 1.0,
            tune: 0.0,
            gain: 1.0,
            pan: 0.0,
            veltrack: 1.0,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: len,
            offset: 0,
            end: len,
            attack: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 0.25,
            group: 0,
            off_by: 0,
            lorand: 0.0,
            hirand: 1.0,
            seq_length: 1,
            seq_position: 1,
        }
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
    region: usize,
    pitch: u8,
    stage: Stage,
    pos: f64,
    step: f64,
    level: f32,
    release_step: f32,
    gain_l: f32,
    gain_r: f32,
    age: u64,
}

impl Voice {
    const IDLE: Voice = Voice {
        region: 0,
        pitch: 0,
        stage: Stage::Idle,
        pos: 0.0,
        step: 0.0,
        level: 0.0,
        release_step: 0.0,
        gain_l: 0.0,
        gain_r: 0.0,
        age: 0,
    };
}

pub struct Sampler {
    regions: Arc<Vec<Region>>,
    sr: f32,
    voices: [Voice; VOICES],
    counter: u64,
    seq: [u32; 128],
    rng: u32,
}

impl Sampler {
    pub fn new(regions: Arc<Vec<Region>>, sample_rate: u32) -> Sampler {
        Sampler {
            regions,
            sr: sample_rate as f32,
            voices: [Voice::IDLE; VOICES],
            counter: 0,
            seq: [0; 128],
            rng: 0x9e37_79b9,
        }
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f32 / u32::MAX as f32
    }

    fn start_release(v: &mut Voice, seconds: f32, sr: f32) {
        v.stage = Stage::Release;
        v.release_step = v.level / (seconds.max(MIN_RELEASE) * sr);
    }

    pub fn note_on(&mut self, pitch: u8, velocity: u8) {
        let velocity = velocity.clamp(1, 127);
        let rand = self.random();
        let count = self.seq[pitch as usize & 127];
        self.seq[pitch as usize & 127] = count.wrapping_add(1);
        let regions = Arc::clone(&self.regions);
        for (ri, r) in regions.iter().enumerate() {
            if pitch < r.lokey || pitch > r.hikey || velocity < r.lovel || velocity > r.hivel {
                continue;
            }
            if rand < r.lorand || (r.hirand < 1.0 && rand >= r.hirand) {
                continue;
            }
            if r.seq_length > 1 && (count % r.seq_length) + 1 != r.seq_position {
                continue;
            }
            if r.sample.is_empty() || r.offset >= r.end {
                continue;
            }
            // Groups: starting this region silences voices it is "off by".
            if r.group != 0 {
                let sr = self.sr;
                for v in &mut self.voices {
                    if v.stage != Stage::Idle && regions[v.region].off_by == r.group {
                        Self::start_release(v, CHOKE_SECONDS, sr);
                    }
                }
            }
            self.counter += 1;
            let slot = self
                .voices
                .iter()
                .position(|v| v.stage == Stage::Idle)
                .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap());
            let semis = (pitch as f32 - r.key_center) * r.keytrack + r.tune;
            let step = r.sample.rate as f64 / self.sr as f64 * 2f64.powf(semis as f64 / 12.0);
            let v_norm = velocity as f32 / 127.0;
            let vel_gain = 1.0 - r.veltrack * (1.0 - v_norm * v_norm);
            let angle = (r.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let g = r.gain * vel_gain * std::f32::consts::SQRT_2;
            self.voices[slot] = Voice {
                region: ri,
                pitch,
                stage: Stage::Attack,
                pos: r.offset as f64,
                step,
                level: 0.0,
                release_step: 0.0,
                gain_l: g * angle.cos(),
                gain_r: g * angle.sin(),
                age: self.counter,
            };
        }
    }

    pub fn note_off(&mut self, pitch: u8) {
        let sr = self.sr;
        for v in &mut self.voices {
            if v.pitch != pitch || matches!(v.stage, Stage::Idle | Stage::Release) {
                continue;
            }
            let r = &self.regions[v.region];
            if r.loop_mode == LoopMode::OneShot {
                continue;
            }
            Self::start_release(v, r.release, sr);
        }
    }

    pub fn all_notes_off(&mut self) {
        let sr = self.sr;
        for v in &mut self.voices {
            if !matches!(v.stage, Stage::Idle | Stage::Release) {
                Self::start_release(v, CHOKE_SECONDS * 3.0, sr);
            }
        }
    }

    #[cfg(test)]
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.stage != Stage::Idle)
            .count()
    }

    /// Stereo; adds into `left` / `right`.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sr;
        let regions = &self.regions;
        for v in &mut self.voices {
            if v.stage == Stage::Idle {
                continue;
            }
            let r = &regions[v.region];
            let data = &r.sample.frames;
            let len = r.end.min(r.sample.len());
            let looping = match r.loop_mode {
                LoopMode::Continuous => true,
                LoopMode::Sustain => v.stage != Stage::Release,
                _ => false,
            } && r.loop_end > r.loop_start + 1
                && r.loop_end <= len;
            let attack_step = 1.0 / (r.attack * sr).max(1.0);
            let decay_step = (1.0 - r.sustain) / (r.decay * sr).max(1.0);
            for i in 0..left.len() {
                match v.stage {
                    Stage::Attack => {
                        v.level += attack_step;
                        if v.level >= 1.0 {
                            v.level = 1.0;
                            v.stage = if r.sustain < 1.0 {
                                Stage::Decay
                            } else {
                                Stage::Sustain
                            };
                        }
                    }
                    Stage::Decay => {
                        v.level -= decay_step;
                        if v.level <= r.sustain {
                            v.level = r.sustain;
                            v.stage = Stage::Sustain;
                        }
                    }
                    Stage::Sustain => {}
                    Stage::Release => {
                        v.level -= v.release_step;
                        if v.level <= 0.0 {
                            *v = Voice::IDLE;
                            break;
                        }
                    }
                    Stage::Idle => break,
                }
                if looping && v.pos >= r.loop_end as f64 {
                    v.pos -= (r.loop_end - r.loop_start) as f64;
                }
                let idx = v.pos as usize;
                if idx + 1 >= len {
                    *v = Voice::IDLE;
                    break;
                }
                let frac = (v.pos - idx as f64) as f32;
                let (a, b) = (idx * 2, idx * 2 + 2);
                let l = data[a] + (data[b] - data[a]) * frac;
                let rr = data[a + 1] + (data[b + 1] - data[a + 1]) * frac;
                left[i] += l * v.level * v.gain_l;
                right[i] += rr * v.level * v.gain_r;
                v.pos += v.step;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, frames: usize) -> Arc<SampleData> {
        let mut d = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = (i as f32 * 440.0 / rate as f32 * std::f32::consts::TAU).sin() * 0.5;
            d.push(s);
            d.push(s);
        }
        Arc::new(SampleData { rate, frames: d })
    }

    fn render(s: &mut Sampler, n: usize) -> Vec<f32> {
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        s.render(&mut l, &mut r);
        l
    }

    fn peak(b: &[f32]) -> f32 {
        b.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    #[test]
    fn plays_and_ends_with_sample() {
        let mut s = Sampler::new(
            Arc::new(vec![Region::whole(sine(48_000, 4800), 60)]),
            48_000,
        );
        assert!(peak(&render(&mut s, 512)) == 0.0);
        s.note_on(60, 127);
        assert!(peak(&render(&mut s, 2400)) > 0.3);
        render(&mut s, 4800);
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn octave_up_plays_twice_as_fast() {
        let mut s = Sampler::new(
            Arc::new(vec![Region::whole(sine(48_000, 4800), 60)]),
            48_000,
        );
        s.note_on(72, 127);
        render(&mut s, 2450);
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn key_and_velocity_ranges_pick_regions() {
        let mut low = Region::whole(sine(48_000, 48_000), 60);
        low.hikey = 59;
        let mut soft = Region::whole(sine(48_000, 48_000), 60);
        soft.lokey = 60;
        soft.hivel = 63;
        let mut loud = soft.clone();
        loud.lovel = 64;
        loud.hivel = 127;
        let mut s = Sampler::new(Arc::new(vec![low, soft, loud]), 48_000);
        s.note_on(50, 100);
        assert_eq!(s.active_voices(), 1);
        s.note_on(70, 30);
        s.note_on(70, 100);
        assert_eq!(s.active_voices(), 3);
        s.note_on(10, 100);
        assert_eq!(s.active_voices(), 4);
    }

    #[test]
    fn loop_sustains_until_release() {
        let mut r = Region::whole(sine(48_000, 4800), 60);
        r.loop_mode = LoopMode::Sustain;
        r.loop_start = 1000;
        r.loop_end = 4000;
        r.release = 0.05;
        let mut s = Sampler::new(Arc::new(vec![r]), 48_000);
        s.note_on(60, 127);
        render(&mut s, 48_000);
        assert_eq!(s.active_voices(), 1);
        s.note_off(60);
        render(&mut s, 4800);
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn one_shot_ignores_note_off_and_off_by_chokes() {
        let mut open = Region::whole(sine(48_000, 48_000), 60);
        open.loop_mode = LoopMode::OneShot;
        open.lokey = 46;
        open.hikey = 46;
        open.off_by = 1;
        let mut closed = open.clone();
        closed.lokey = 42;
        closed.hikey = 42;
        closed.group = 1;
        closed.off_by = 0;
        let mut s = Sampler::new(Arc::new(vec![open, closed]), 48_000);
        s.note_on(46, 100);
        s.note_off(46);
        render(&mut s, 100);
        assert_eq!(s.active_voices(), 1);
        s.note_on(42, 100);
        render(&mut s, 2000);
        assert_eq!(s.active_voices(), 1, "open hat choked, closed hat playing");
    }

    #[test]
    fn round_robin_alternates() {
        let mut a = Region::whole(sine(48_000, 48_000), 60);
        a.seq_length = 2;
        a.seq_position = 1;
        let mut b = a.clone();
        b.seq_position = 2;
        let mut s = Sampler::new(Arc::new(vec![a, b]), 48_000);
        s.note_on(60, 100);
        s.note_on(60, 100);
        assert_eq!(s.active_voices(), 2);
        let regions: Vec<usize> = s
            .voices
            .iter()
            .filter(|v| v.stage != Stage::Idle)
            .map(|v| v.region)
            .collect();
        assert_eq!(regions, vec![0, 1]);
    }
}
