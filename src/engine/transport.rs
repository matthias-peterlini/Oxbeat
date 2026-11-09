//! Converting between musical time (beats) and audio time (samples).

pub fn samples_per_beat(tempo_bpm: f64, sample_rate: u32) -> f64 {
    sample_rate as f64 * 60.0 / tempo_bpm
}

/// The sample on which `beat` starts (rounded down).
pub fn beat_to_sample(beat: f64, tempo_bpm: f64, sample_rate: u32) -> u64 {
    // Small epsilon guards against 3.9999999 -> 3 floating point drift.
    (beat * samples_per_beat(tempo_bpm, sample_rate) + 1e-6)
        .floor()
        .max(0.0) as u64
}

pub fn sample_to_beat(sample: u64, tempo_bpm: f64, sample_rate: u32) -> f64 {
    sample as f64 / samples_per_beat(tempo_bpm, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(samples_per_beat(120.0, 48_000), 24_000.0);
        assert_eq!(beat_to_sample(4.0, 120.0, 44_100), 88_200);
        assert!((sample_to_beat(88_200, 120.0, 44_100) - 4.0).abs() < 1e-9);
        assert_eq!(beat_to_sample(1.0, 90.0, 48_000), 32_000);
    }

    #[test]
    fn roundtrip_at_odd_tempo() {
        for s in [0u64, 1, 12_345, 1_000_000] {
            let b = sample_to_beat(s, 97.3, 44_100);
            assert_eq!(beat_to_sample(b, 97.3, 44_100), s);
        }
    }
}
