//! Turning sound ids into playable instruments: built-ins, imported files, packs.

use crate::engine::instrument::Instrument;

/// Built-in instruments: (sound id, caption).
pub const BUILTIN_INSTRUMENTS: [(&str, &str); 4] = [
    ("Keys", "Soft electric piano"),
    ("Bass", "Warm, round synth bass"),
    ("Lead", "Bright square lead"),
    ("Pad", "Slow, wide synth pad"),
];

/// Built-in drum kits: (sound id, caption).
pub const BUILTIN_KITS: [(&str, &str); 2] = [
    ("Studio Kit", "Punchy acoustic-style kit"),
    ("808 Kit", "Deep electronic kick and crisp hats"),
];

/// Build the instrument for a sound id at the engine's sample rate.
pub fn load(sound: &str, sample_rate: u32) -> Result<Instrument, String> {
    Instrument::builtin(sound, sample_rate).ok_or_else(|| "unknown sound".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_loads() {
        for (id, _) in BUILTIN_INSTRUMENTS.iter().chain(&BUILTIN_KITS) {
            assert!(load(id, 48_000).is_ok(), "{id}");
        }
        assert!(load("nope", 48_000).is_err());
    }
}
