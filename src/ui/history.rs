//! Undo and redo: snapshots of the project, one per gesture.

use std::time::{Duration, Instant};

use crate::project::Project;

/// Changes closer together than this are one gesture (dragging a fader changes the
/// project every frame; undo should take back the whole drag).
const GESTURE_GAP: Duration = Duration::from_millis(400);
const MAX_STEPS: usize = 200;

pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// The project as last seen.
    current: Project,
    last_change: Option<Instant>,
}

impl History {
    pub fn new(project: &Project) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            current: project.clone(),
            last_change: None,
        }
    }

    /// Forget everything (a different project was opened).
    pub fn reset(&mut self, project: &Project) {
        *self = Self::new(project);
    }

    /// Note the project as it is at `now`. A change starts a new undo step unless it
    /// continues the gesture before it.
    pub fn record(&mut self, project: &Project, now: Instant) {
        if *project == self.current {
            return;
        }
        let continues = self
            .last_change
            .is_some_and(|t| now.saturating_duration_since(t) <= GESTURE_GAP);
        let before = std::mem::replace(&mut self.current, project.clone());
        if !continues {
            self.undo.push(before);
            if self.undo.len() > MAX_STEPS {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_change = Some(now);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The project to go back to, if any.
    pub fn undo(&mut self) -> Option<Project> {
        let previous = self.undo.pop()?;
        self.redo
            .push(std::mem::replace(&mut self.current, previous.clone()));
        self.last_change = None;
        Some(previous)
    }

    pub fn redo(&mut self) -> Option<Project> {
        let next = self.redo.pop()?;
        self.undo
            .push(std::mem::replace(&mut self.current, next.clone()));
        self.last_change = None;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Project {
        let mut p = Project::default();
        p.name = name.into();
        p
    }

    #[test]
    fn undo_and_redo_walk_through_changes() {
        let t0 = Instant::now();
        let step = Duration::from_secs(1);
        let mut h = History::new(&named("a"));
        assert!(!h.can_undo());
        h.record(&named("b"), t0);
        h.record(&named("c"), t0 + step);
        assert_eq!(h.undo().unwrap().name, "b");
        assert_eq!(h.undo().unwrap().name, "a");
        assert!(h.undo().is_none());
        assert_eq!(h.redo().unwrap().name, "b");
        assert_eq!(h.redo().unwrap().name, "c");
        assert!(h.redo().is_none());
    }

    #[test]
    fn a_drag_is_one_step() {
        let t0 = Instant::now();
        let frame = Duration::from_millis(16);
        let mut h = History::new(&named("start"));
        for i in 0..30u32 {
            h.record(&named(&format!("drag {i}")), t0 + frame * i);
        }
        assert_eq!(h.undo().unwrap().name, "start");
        assert!(!h.can_undo());
    }

    #[test]
    fn unchanged_project_records_nothing() {
        let mut h = History::new(&named("a"));
        h.record(&named("a"), Instant::now());
        assert!(!h.can_undo());
    }

    #[test]
    fn a_new_change_after_undo_drops_redo() {
        let t0 = Instant::now();
        let mut h = History::new(&named("a"));
        h.record(&named("b"), t0);
        h.undo();
        // Straight after an undo, even a quick change is its own step.
        h.record(&named("c"), t0 + Duration::from_millis(10));
        assert!(!h.can_redo());
        assert_eq!(h.undo().unwrap().name, "a");
    }

    #[test]
    fn history_is_bounded() {
        let t0 = Instant::now();
        let mut h = History::new(&named("0"));
        for i in 1..=(MAX_STEPS as u32 + 50) {
            h.record(&named(&i.to_string()), t0 + Duration::from_secs(i as u64));
        }
        let mut steps = 0;
        while h.undo().is_some() {
            steps += 1;
        }
        assert_eq!(steps, MAX_STEPS);
    }
}
