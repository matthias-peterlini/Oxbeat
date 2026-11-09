//! Play notes with the computer keyboard (piano layout on the home row).

use egui::Key;

/// White keys on A S D F G H J K, black keys on W E T Y U.
const LAYOUT: [Key; 13] = [
    Key::A,
    Key::W,
    Key::S,
    Key::E,
    Key::D,
    Key::F,
    Key::T,
    Key::G,
    Key::Y,
    Key::H,
    Key::U,
    Key::J,
    Key::K,
];

pub const MIN_OCTAVE: i8 = 0;
pub const MAX_OCTAVE: i8 = 8;

/// MIDI pitch for `key` with `octave` as the C octave (octave 4: A → 60, middle C).
pub fn key_to_pitch(key: Key, octave: i8) -> Option<u8> {
    let semitone = LAYOUT.iter().position(|k| *k == key)? as i32;
    let pitch = (octave as i32 + 1) * 12 + semitone;
    u8::try_from(pitch).ok().filter(|p| *p <= 127)
}

/// Drum pad index (0-based, in `PADS` order) for `key`: same keys as the piano layout.
pub fn key_to_pad(key: Key) -> Option<usize> {
    PAD_KEYS.iter().position(|k| *k == key)
}

/// Home row first (kick, snare, hats on A S D F), then the row above.
const PAD_KEYS: [Key; 12] = [
    Key::A,
    Key::S,
    Key::D,
    Key::F,
    Key::G,
    Key::H,
    Key::J,
    Key::K,
    Key::W,
    Key::E,
    Key::T,
    Key::Y,
];

/// The key label for pad `index`.
pub fn pad_key_label(index: usize) -> &'static str {
    const LABELS: [&str; 12] = ["A", "S", "D", "F", "G", "H", "J", "K", "W", "E", "T", "Y"];
    LABELS.get(index).copied().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_follow_layout() {
        assert_eq!(key_to_pad(Key::A), Some(0));
        assert_eq!(key_to_pad(Key::S), Some(1));
        assert_eq!(key_to_pad(Key::Y), Some(11));
        assert_eq!(key_to_pad(Key::U), None);
        assert_eq!(pad_key_label(2), "D");
    }

    #[test]
    fn maps_home_row() {
        assert_eq!(key_to_pitch(Key::A, 4), Some(60));
        assert_eq!(key_to_pitch(Key::W, 4), Some(61));
        assert_eq!(key_to_pitch(Key::K, 4), Some(72));
        assert_eq!(key_to_pitch(Key::Q, 4), None);
        assert_eq!(key_to_pitch(Key::A, 3), Some(48));
    }

    #[test]
    fn stays_in_midi_range() {
        assert_eq!(key_to_pitch(Key::K, 9), None);
        assert_eq!(key_to_pitch(Key::A, 8), Some(108));
    }
}

/// Indices of held computer keys that must be released: the key is no longer down,
/// or the window lost focus (no key-up will ever arrive).
pub fn stale_keys(held: &[(Key, u8)], is_down: impl Fn(Key) -> bool, focused: bool) -> Vec<usize> {
    held.iter()
        .enumerate()
        .filter(|(_, (k, _))| !focused || !is_down(*k))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod stale_tests {
    use super::*;

    #[test]
    fn finds_keys_no_longer_down() {
        let held = [(Key::A, 60u8), (Key::S, 62u8)];
        assert_eq!(stale_keys(&held, |k| k == Key::S, true), vec![0]);
        assert_eq!(stale_keys(&held, |_| true, false), vec![0, 1]);
    }
}
