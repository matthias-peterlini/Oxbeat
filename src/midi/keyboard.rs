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

#[cfg(test)]
mod tests {
    use super::*;

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
