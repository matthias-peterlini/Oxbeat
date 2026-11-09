//! Effects on a track: what kinds exist, their knobs, and the chain stored in the project.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectKind {
    Gate,
    PitchCorrect,
    DeEsser,
    Eq,
    Compressor,
    Distortion,
    Chorus,
    Delay,
    Reverb,
}

/// How a knob's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    /// Stored 0.0..=1.0, shown 0..100 %.
    Percent,
    Db,
    Hz,
    Ms,
    /// A compression ratio, shown "4.0 : 1".
    Ratio,
    /// One of a list; the value is the index.
    Choice(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
}

const fn p(name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Param {
    Param {
        name,
        min,
        max,
        default,
        unit,
    }
}

const fn percent(name: &'static str, default: f32) -> Param {
    p(name, 0.0, 1.0, default, Unit::Percent)
}

/// Scales pitch correction can snap to. A minor key shares the notes of its relative
/// major, so major keys cover both.
pub const SCALES: [&str; 13] = [
    "All notes",
    "C major / A minor",
    "Db major / Bb minor",
    "D major / B minor",
    "Eb major / C minor",
    "E major / C# minor",
    "F major / D minor",
    "F# major / D# minor",
    "G major / E minor",
    "Ab major / F minor",
    "A major / F# minor",
    "Bb major / G minor",
    "B major / G# minor",
];

impl EffectKind {
    /// Every kind, in the order a typical vocal chain uses them.
    pub const ALL: [EffectKind; 9] = [
        EffectKind::Gate,
        EffectKind::PitchCorrect,
        EffectKind::DeEsser,
        EffectKind::Eq,
        EffectKind::Compressor,
        EffectKind::Distortion,
        EffectKind::Chorus,
        EffectKind::Delay,
        EffectKind::Reverb,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Gate => "Noise Gate",
            EffectKind::PitchCorrect => "Pitch Correction",
            EffectKind::DeEsser => "De-esser",
            EffectKind::Eq => "Equalizer",
            EffectKind::Compressor => "Compressor",
            EffectKind::Distortion => "Distortion",
            EffectKind::Chorus => "Chorus",
            EffectKind::Delay => "Delay",
            EffectKind::Reverb => "Reverb",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            EffectKind::Gate => "Silences hiss between phrases",
            EffectKind::PitchCorrect => "Nudges notes into tune",
            EffectKind::DeEsser => "Softens harsh S sounds",
            EffectKind::Eq => "Shape bass, middle and treble",
            EffectKind::Compressor => "Evens out loud and quiet",
            EffectKind::Distortion => "Grit, warmth or crunch",
            EffectKind::Chorus => "Thickens and widens",
            EffectKind::Delay => "Echoes",
            EffectKind::Reverb => "Puts the sound in a room",
        }
    }

    pub fn params(self) -> &'static [Param] {
        const GATE: [Param; 2] = [
            p("Threshold", -80.0, 0.0, -45.0, Unit::Db),
            p("Release", 20.0, 1000.0, 150.0, Unit::Ms),
        ];
        const PITCH: [Param; 3] = [
            percent("Strength", 1.0),
            p("Speed", 0.0, 400.0, 40.0, Unit::Ms),
            p("Scale", 0.0, 12.0, 0.0, Unit::Choice(&SCALES)),
        ];
        const DEESS: [Param; 2] = [
            p("Frequency", 3000.0, 10_000.0, 6000.0, Unit::Hz),
            percent("Amount", 0.5),
        ];
        const EQ: [Param; 4] = [
            p("Low", -15.0, 15.0, 0.0, Unit::Db),
            p("Mid", -15.0, 15.0, 0.0, Unit::Db),
            p("Mid Freq", 200.0, 5000.0, 1000.0, Unit::Hz),
            p("High", -15.0, 15.0, 0.0, Unit::Db),
        ];
        const COMP: [Param; 5] = [
            p("Threshold", -60.0, 0.0, -18.0, Unit::Db),
            p("Ratio", 1.0, 20.0, 4.0, Unit::Ratio),
            p("Attack", 0.1, 100.0, 10.0, Unit::Ms),
            p("Release", 10.0, 1000.0, 120.0, Unit::Ms),
            p("Gain", 0.0, 24.0, 0.0, Unit::Db),
        ];
        const DIST: [Param; 3] = [
            percent("Drive", 0.5),
            percent("Tone", 0.6),
            percent("Mix", 1.0),
        ];
        const CHORUS: [Param; 3] = [
            p("Rate", 0.1, 5.0, 0.8, Unit::Hz),
            percent("Depth", 0.5),
            percent("Mix", 0.5),
        ];
        const DELAY: [Param; 3] = [
            p("Time", 20.0, 1500.0, 350.0, Unit::Ms),
            p("Feedback", 0.0, 0.95, 0.35, Unit::Percent),
            percent("Mix", 0.3),
        ];
        const REVERB: [Param; 3] = [
            percent("Size", 0.6),
            percent("Damping", 0.5),
            percent("Mix", 0.25),
        ];
        match self {
            EffectKind::Gate => &GATE,
            EffectKind::PitchCorrect => &PITCH,
            EffectKind::DeEsser => &DEESS,
            EffectKind::Eq => &EQ,
            EffectKind::Compressor => &COMP,
            EffectKind::Distortion => &DIST,
            EffectKind::Chorus => &CHORUS,
            EffectKind::Delay => &DELAY,
            EffectKind::Reverb => &REVERB,
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Effect {
    pub kind: EffectKind,
    #[serde(default = "yes")]
    pub on: bool,
    /// One value per knob of `kind.params()`, in real units.
    #[serde(default)]
    pub params: Vec<f32>,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            on: true,
            params: kind.params().iter().map(|p| p.default).collect(),
        }
    }

    /// `kind` with some knobs set: `(knob index, value)`.
    pub fn with(kind: EffectKind, values: &[(usize, f32)]) -> Self {
        let mut e = Self::new(kind);
        for (i, v) in values {
            e.set(*i, *v);
        }
        e
    }

    /// Knob `i`, always inside its range (files may hold anything).
    pub fn get(&self, i: usize) -> f32 {
        let Some(spec) = self.kind.params().get(i) else {
            return 0.0;
        };
        match self.params.get(i) {
            Some(v) if v.is_finite() => v.clamp(spec.min, spec.max),
            _ => spec.default,
        }
    }

    pub fn set(&mut self, i: usize, value: f32) {
        let specs = self.kind.params();
        let Some(spec) = specs.get(i) else {
            return;
        };
        if self.params.len() < specs.len() {
            let from = self.params.len();
            self.params.extend(specs[from..].iter().map(|p| p.default));
        }
        self.params[i] = value.clamp(spec.min, spec.max);
    }
}

/// Ready-made chains, so a voice sounds finished in one click.
pub const PRESETS: [&str; 5] = [
    "Clean Vocal",
    "Radio Voice",
    "Big Hall",
    "Tuned Pop",
    "Telephone",
];

pub fn preset(name: &str) -> Vec<Effect> {
    use EffectKind::*;
    match name {
        "Clean Vocal" => vec![
            Effect::new(Gate),
            Effect::with(Eq, &[(0, -3.0), (3, 3.0)]),
            Effect::new(Compressor),
            Effect::with(Reverb, &[(0, 0.4), (2, 0.15)]),
        ],
        "Radio Voice" => vec![
            Effect::new(Gate),
            Effect::new(DeEsser),
            Effect::with(Eq, &[(0, 4.0), (1, -2.0), (2, 400.0), (3, 4.0)]),
            Effect::with(Compressor, &[(0, -26.0), (1, 6.0), (4, 8.0)]),
        ],
        "Big Hall" => vec![
            Effect::new(Compressor),
            Effect::with(Delay, &[(0, 420.0), (1, 0.3), (2, 0.18)]),
            Effect::with(Reverb, &[(0, 0.9), (1, 0.3), (2, 0.4)]),
        ],
        "Tuned Pop" => vec![
            Effect::with(PitchCorrect, &[(1, 0.0)]),
            Effect::new(DeEsser),
            Effect::with(Compressor, &[(0, -24.0), (1, 5.0), (4, 6.0)]),
            Effect::with(Delay, &[(0, 250.0), (1, 0.2), (2, 0.12)]),
            Effect::with(Reverb, &[(0, 0.5), (2, 0.18)]),
        ],
        "Telephone" => vec![
            Effect::with(Eq, &[(0, -15.0), (1, 12.0), (2, 1800.0), (3, -15.0)]),
            Effect::with(Distortion, &[(0, 0.45), (1, 0.35)]),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_effect_has_default_for_every_knob() {
        for kind in EffectKind::ALL {
            let e = Effect::new(kind);
            assert_eq!(e.params.len(), kind.params().len());
            for (i, spec) in kind.params().iter().enumerate() {
                assert!(spec.min <= spec.default && spec.default <= spec.max);
                assert_eq!(e.get(i), spec.default);
            }
        }
    }

    #[test]
    fn values_from_files_are_clamped_and_filled_in() {
        let e: Effect = serde_json::from_str(r#"{"kind":"Reverb","params":[7.0]}"#).unwrap();
        assert!(e.on);
        assert_eq!(e.get(0), 1.0);
        assert_eq!(e.get(2), 0.25);
        let nan = Effect {
            kind: EffectKind::Delay,
            on: true,
            params: vec![f32::NAN],
        };
        assert_eq!(nan.get(0), 350.0);
    }

    #[test]
    fn every_preset_builds() {
        for name in PRESETS {
            assert!(!preset(name).is_empty(), "{name}");
        }
    }
}
