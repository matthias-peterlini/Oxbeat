//! Editing operations on clips and notes.

use super::{Clip, Note, Project, Track};

/// Shortest a clip or note may be made, in beats.
pub const MIN_LENGTH: f64 = 1.0 / 16.0;

/// The nearest multiple of `grid` (a grid of 0 leaves the value alone).
pub fn snap(beat: f64, grid: f64) -> f64 {
    if grid <= 0.0 {
        beat
    } else {
        (beat / grid).round() * grid
    }
}

impl Clip {
    pub fn end_beat(&self) -> f64 {
        self.start_beat + self.length_beats
    }

    /// Move every note onto the nearest grid line, keeping its length.
    pub fn quantize(&mut self, grid: f64) {
        let last = (self.length_beats - grid).max(0.0);
        for n in &mut self.notes {
            n.start_beat = snap(n.start_beat, grid).clamp(0.0, last);
        }
    }
}

impl Project {
    /// A clip and the track it is on.
    pub fn clip(&self, id: u64) -> Option<(&Track, &Clip)> {
        self.tracks
            .iter()
            .find_map(|t| t.clips.iter().find(|c| c.id == id).map(|c| (t, c)))
    }

    pub fn clip_mut(&mut self, id: u64) -> Option<&mut Clip> {
        self.tracks
            .iter_mut()
            .flat_map(|t| &mut t.clips)
            .find(|c| c.id == id)
    }

    pub fn remove_clip(&mut self, id: u64) -> bool {
        for t in &mut self.tracks {
            let before = t.clips.len();
            t.clips.retain(|c| c.id != id);
            if t.clips.len() != before {
                return true;
            }
        }
        false
    }

    pub fn remove_track(&mut self, id: u64) -> bool {
        let before = self.tracks.len();
        self.tracks.retain(|t| t.id != id);
        self.tracks.len() != before
    }

    /// Copy a clip to right after itself. Returns the copy's id.
    pub fn duplicate_clip(&mut self, id: u64) -> Option<u64> {
        let new_id = self.new_id();
        let track = self
            .tracks
            .iter_mut()
            .find(|t| t.clips.iter().any(|c| c.id == id))?;
        let mut copy = track.clips.iter().find(|c| c.id == id)?.clone();
        copy.id = new_id;
        copy.start_beat = copy.end_beat();
        track.clips.push(copy);
        Some(new_id)
    }

    /// Cut a clip in two at `beat` (song time). Returns the id of the second half.
    pub fn split_clip(&mut self, id: u64, beat: f64) -> Option<u64> {
        let new_id = self.new_id();
        let seconds_per_beat = 60.0 / self.tempo_bpm.max(1.0);
        let track = self
            .tracks
            .iter_mut()
            .find(|t| t.clips.iter().any(|c| c.id == id))?;
        let first = track.clips.iter_mut().find(|c| c.id == id)?;
        let at = beat - first.start_beat;
        if at < MIN_LENGTH || at > first.length_beats - MIN_LENGTH {
            return None;
        }
        let mut second = Clip {
            id: new_id,
            start_beat: beat,
            length_beats: first.length_beats - at,
            notes: Vec::new(),
            audio: first.audio.clone(),
            audio_offset: first.audio_offset + at * seconds_per_beat,
        };
        first.length_beats = at;
        let (keep, moved): (Vec<Note>, Vec<Note>) =
            first.notes.iter().copied().partition(|n| n.start_beat < at);
        first.notes = keep;
        for n in &mut first.notes {
            // A note held across the cut ends at it.
            n.length_beats = n.length_beats.min(at - n.start_beat);
        }
        second.notes = moved
            .into_iter()
            .map(|n| Note {
                start_beat: n.start_beat - at,
                ..n
            })
            .collect();
        track.clips.push(second);
        Some(new_id)
    }

    /// An empty clip for drawing notes into. Returns its id.
    pub fn add_empty_clip(
        &mut self,
        track: u64,
        start_beat: f64,
        length_beats: f64,
    ) -> Option<u64> {
        let id = self.new_id();
        let t = self.track_mut(track)?;
        if t.is_audio() {
            return None;
        }
        t.clips.push(Clip {
            id,
            start_beat: start_beat.max(0.0),
            length_beats: length_beats.max(MIN_LENGTH),
            ..Clip::default()
        });
        Some(id)
    }

    /// Change the tempo. Recordings keep their real length, so audio clips cover a
    /// different number of beats afterwards.
    pub fn set_tempo(&mut self, bpm: f64) {
        if bpm <= 0.0 || bpm == self.tempo_bpm {
            return;
        }
        let scale = bpm / self.tempo_bpm;
        for clip in self.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if clip.is_audio() {
                clip.length_beats *= scale;
            }
        }
        self.tempo_bpm = bpm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, start: f64, len: f64) -> Note {
        Note {
            pitch,
            velocity: 100,
            start_beat: start,
            length_beats: len,
        }
    }

    /// A project with one 8-beat clip of three notes on a Keys track.
    fn song() -> (Project, u64, u64) {
        let mut p = Project::default();
        let track = p.add_instrument_track("Keys");
        let clip = p.add_empty_clip(track, 4.0, 8.0).unwrap();
        p.clip_mut(clip).unwrap().notes =
            vec![note(60, 0.0, 1.0), note(62, 3.5, 2.0), note(64, 6.1, 1.0)];
        (p, track, clip)
    }

    #[test]
    fn snaps_to_the_grid() {
        assert_eq!(snap(1.1, 0.25), 1.0);
        assert_eq!(snap(1.13, 0.25), 1.25);
        assert_eq!(snap(1.13, 0.0), 1.13);
    }

    #[test]
    fn quantize_moves_starts_and_keeps_lengths() {
        let (mut p, _, clip) = song();
        let c = p.clip_mut(clip).unwrap();
        c.notes.push(note(65, 7.95, 0.5));
        c.quantize(0.5);
        let starts: Vec<f64> = c.notes.iter().map(|n| n.start_beat).collect();
        // The last note would land on the clip's end; it stays inside instead.
        assert_eq!(starts, vec![0.0, 3.5, 6.0, 7.5]);
        assert_eq!(c.notes[2].length_beats, 1.0);
    }

    #[test]
    fn duplicate_lands_right_after() {
        let (mut p, track, clip) = song();
        let copy = p.duplicate_clip(clip).unwrap();
        assert_ne!(copy, clip);
        let (t, c) = p.clip(copy).unwrap();
        assert_eq!(t.id, track);
        assert_eq!((c.start_beat, c.length_beats), (12.0, 8.0));
        assert_eq!(c.notes.len(), 3);
        assert_eq!(p.duplicate_clip(999), None);
    }

    #[test]
    fn split_divides_notes_and_cuts_held_ones() {
        let (mut p, _, clip) = song();
        let second = p.split_clip(clip, 8.0).unwrap();
        let first = p.clip(clip).unwrap().1.clone();
        let second = p.clip(second).unwrap().1.clone();
        assert_eq!((first.start_beat, first.length_beats), (4.0, 4.0));
        assert_eq!((second.start_beat, second.length_beats), (8.0, 4.0));
        // The note at 3.5 lasted 2 beats; it now stops at the cut.
        assert_eq!(first.notes, vec![note(60, 0.0, 1.0), note(62, 3.5, 0.5)]);
        assert_eq!(second.notes.len(), 1);
        assert!((second.notes[0].start_beat - 2.1).abs() < 1e-9);
    }

    #[test]
    fn split_outside_the_clip_does_nothing() {
        let (mut p, _, clip) = song();
        let before = p.clone();
        assert_eq!(p.split_clip(clip, 4.0), None);
        assert_eq!(p.split_clip(clip, 12.0), None);
        assert_eq!(p.split_clip(clip, 50.0), None);
        assert_eq!(p.tracks, before.tracks);
    }

    #[test]
    fn splitting_audio_starts_the_second_half_later_in_the_file() {
        let mut p = Project::default();
        let track = p.add_voice_track();
        let id = p.new_id();
        p.track_mut(track).unwrap().clips.push(Clip {
            id,
            start_beat: 2.0,
            length_beats: 8.0,
            audio: Some("take.wav".into()),
            audio_offset: 1.0,
            ..Clip::default()
        });
        // 120 BPM: 3 beats in is 1.5 s.
        let second = p.split_clip(id, 5.0).unwrap();
        let (_, c) = p.clip(second).unwrap();
        assert_eq!(c.audio.as_deref(), Some("take.wav"));
        assert_eq!(c.audio_offset, 2.5);
        assert_eq!(c.length_beats, 5.0);
    }

    #[test]
    fn remove_clip_and_track() {
        let (mut p, track, clip) = song();
        assert!(p.remove_clip(clip));
        assert!(!p.remove_clip(clip));
        assert!(p.remove_track(track));
        assert!(p.tracks.is_empty());
    }

    #[test]
    fn empty_clips_are_for_note_tracks_only() {
        let mut p = Project::default();
        let voice = p.add_voice_track();
        assert_eq!(p.add_empty_clip(voice, 0.0, 4.0), None);
        assert_eq!(p.add_empty_clip(999, 0.0, 4.0), None);
    }

    #[test]
    fn tempo_change_rescales_audio_clips_only() {
        let (mut p, _, notes) = song();
        let voice = p.add_voice_track();
        let id = p.new_id();
        p.track_mut(voice).unwrap().clips.push(Clip {
            id,
            length_beats: 8.0,
            audio: Some("take.wav".into()),
            ..Clip::default()
        });
        p.set_tempo(60.0);
        assert_eq!(p.tempo_bpm, 60.0);
        // 4 seconds of audio is 8 beats at 120 and 4 beats at 60.
        assert_eq!(p.clip(id).unwrap().1.length_beats, 4.0);
        assert_eq!(p.clip(notes).unwrap().1.length_beats, 8.0);
    }
}
