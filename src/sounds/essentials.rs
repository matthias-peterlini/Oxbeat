//! The "essentials" instrument pack downloads itself on first launch, unless the
//! user deliberately removed it. This file holds that decision and its saved state.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const ID: &str = "essentials";
const FILE: &str = "essentials.json";

#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
pub struct State {
    pub removed_by_user: bool,
}

/// Reads `dir/essentials.json`; the default when missing or corrupt.
pub fn load(dir: &Path) -> State {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, s: &State) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string(s) {
        let _ = std::fs::write(dir.join(FILE), text);
    }
}

pub fn should_auto_install(installed: bool, downloading: bool, state: &State) -> bool {
    !installed && !downloading && !state.removed_by_user
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_install_only_when_missing_and_not_removed() {
        let keep = State {
            removed_by_user: false,
        };
        let removed = State {
            removed_by_user: true,
        };
        assert!(should_auto_install(false, false, &keep));
        assert!(!should_auto_install(true, false, &keep));
        assert!(!should_auto_install(false, true, &keep));
        assert!(!should_auto_install(true, true, &keep));
        assert!(!should_auto_install(false, false, &removed));
        assert!(!should_auto_install(true, false, &removed));
    }

    #[test]
    fn state_round_trips_and_survives_garbage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), State::default());
        let s = State {
            removed_by_user: true,
        };
        save(dir.path(), &s);
        assert_eq!(load(dir.path()), s);
        std::fs::write(dir.path().join(FILE), "{{ not json").unwrap();
        assert_eq!(load(dir.path()), State::default());
    }
}
