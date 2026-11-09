//! MIDI keyboards: connect to every input port, follow hot-plugging.

use std::collections::HashMap;
use std::sync::Arc;

use midir::{MidiInput, MidiInputConnection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMsg {
    NoteOn { pitch: u8, velocity: u8 },
    NoteOff { pitch: u8 },
}

/// Decode a raw MIDI message (any channel). Non-note messages yield `None`.
pub fn parse(bytes: &[u8]) -> Option<MidiMsg> {
    let [status, pitch, velocity, ..] = *bytes else {
        return None;
    };
    let (pitch, velocity) = (pitch & 0x7f, velocity & 0x7f);
    match status & 0xf0 {
        0x90 if velocity > 0 => Some(MidiMsg::NoteOn { pitch, velocity }),
        0x90 | 0x80 => Some(MidiMsg::NoteOff { pitch }),
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct RescanResult {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

pub fn diff_ports(old: &[String], new: &[String]) -> RescanResult {
    RescanResult {
        added: new.iter().filter(|p| !old.contains(p)).cloned().collect(),
        removed: old.iter().filter(|p| !new.contains(p)).cloned().collect(),
    }
}

pub type MidiSink = Arc<dyn Fn(MidiMsg) + Send + Sync>;

pub struct MidiInputs {
    sink: MidiSink,
    /// port id -> (display name, live connection)
    connections: HashMap<String, (String, MidiInputConnection<()>)>,
}

const CLIENT: &str = "Oxbeat";

impl MidiInputs {
    pub fn new(sink: MidiSink) -> Self {
        Self {
            sink,
            connections: HashMap::new(),
        }
    }

    /// Names of currently connected keyboards.
    pub fn connected(&self) -> Vec<String> {
        let mut v: Vec<String> = self.connections.values().map(|(n, _)| n.clone()).collect();
        v.sort();
        v
    }

    /// Connect new ports and drop vanished ones. Returns display names that changed.
    pub fn rescan(&mut self) -> RescanResult {
        let Ok(probe) = MidiInput::new(CLIENT) else {
            return RescanResult::default();
        };
        let available: Vec<(String, String)> = probe
            .ports()
            .iter()
            .filter_map(|p| Some((p.id(), probe.port_name(p).ok()?)))
            .collect();
        let old_ids: Vec<String> = self.connections.keys().cloned().collect();
        let new_ids: Vec<String> = available.iter().map(|(id, _)| id.clone()).collect();
        let diff = diff_ports(&old_ids, &new_ids);

        let mut result = RescanResult::default();
        for id in &diff.removed {
            if let Some((name, conn)) = self.connections.remove(id) {
                conn.close();
                result.removed.push(name);
            }
        }
        for id in &diff.added {
            let Some((_, name)) = available.iter().find(|(i, _)| i == id) else {
                continue;
            };
            let Ok(input) = MidiInput::new(CLIENT) else {
                continue;
            };
            let Some(port) = input.find_port_by_id(id) else {
                continue;
            };
            let sink = self.sink.clone();
            if let Ok(conn) = input.connect(
                &port,
                "oxbeat-in",
                move |_, bytes, _| {
                    if let Some(msg) = parse(bytes) {
                        sink(msg);
                    }
                },
                (),
            ) {
                self.connections.insert(id.clone(), (name.clone(), conn));
                result.added.push(name.clone());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notes() {
        assert_eq!(
            parse(&[0x90, 60, 100]),
            Some(MidiMsg::NoteOn {
                pitch: 60,
                velocity: 100
            })
        );
        assert_eq!(parse(&[0x93, 60, 0]), Some(MidiMsg::NoteOff { pitch: 60 }));
        assert_eq!(parse(&[0x80, 64, 10]), Some(MidiMsg::NoteOff { pitch: 64 }));
        assert_eq!(parse(&[0xB0, 7, 100]), None);
        assert_eq!(parse(&[0x90]), None);
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn diffs_ports() {
        let r = diff_ports(&["A".into(), "B".into()], &["B".into(), "C".into()]);
        assert_eq!(
            (r.added, r.removed),
            (vec!["C".to_string()], vec!["A".to_string()])
        );
    }
}
