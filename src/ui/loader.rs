//! Builds file and pack instruments on a worker thread, so the window never waits
//! on decoding samples. Jobs run one at a time: the sample cache then serves any
//! later job that shares files with an earlier one.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};

use crate::engine::instrument::Instrument;

/// A finished load: the instrument for `sound` at `sample_rate`.
pub struct Loaded {
    pub sound: String,
    pub sample_rate: u32,
    pub result: Result<Instrument, String>,
}

type Repaint = Arc<OnceLock<Box<dyn Fn() + Send + Sync>>>;

pub struct Loader {
    jobs: Sender<(String, u32)>,
    done: Receiver<Loaded>,
    /// Requested and not yet received, so the same sound isn't loaded twice at once.
    in_flight: HashSet<(String, u32)>,
    repaint: Repaint,
}

impl Loader {
    /// `load` runs on the worker thread.
    pub fn new(
        mut load: impl FnMut(&str, u32) -> Result<Instrument, String> + Send + 'static,
    ) -> Loader {
        let (jobs, job_rx) = channel::<(String, u32)>();
        let (done_tx, done) = channel();
        let repaint: Repaint = Arc::new(OnceLock::new());
        let wake = repaint.clone();
        std::thread::Builder::new()
            .name("instrument loader".into())
            .spawn(move || {
                // Ends when the Loader (and so the job sender) is dropped.
                for (sound, sample_rate) in job_rx {
                    let result = load(&sound, sample_rate);
                    let loaded = Loaded {
                        sound,
                        sample_rate,
                        result,
                    };
                    if done_tx.send(loaded).is_err() {
                        break;
                    }
                    if let Some(r) = wake.get() {
                        r();
                    }
                }
            })
            .expect("could not start the instrument loader");
        Loader {
            jobs,
            done,
            in_flight: HashSet::new(),
            repaint,
        }
    }

    /// The default: `sounds::load`.
    pub fn sounds() -> Loader {
        Loader::new(crate::sounds::load)
    }

    /// Called from the worker after each load (e.g. to wake the UI).
    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.repaint.set(Box::new(f));
    }

    /// Queue a load unless the same one is already on its way.
    pub fn request(&mut self, sound: &str, sample_rate: u32) {
        let key = (sound.to_string(), sample_rate);
        if self.in_flight.contains(&key) {
            return;
        }
        if self.jobs.send(key.clone()).is_ok() {
            self.in_flight.insert(key);
        }
    }

    /// A finished load, if any.
    pub fn try_recv(&mut self) -> Option<Loaded> {
        let l = self.done.try_recv().ok()?;
        self.in_flight.remove(&(l.sound.clone(), l.sample_rate));
        Some(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn same_sound_is_loaded_once_while_in_flight() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let mut l = Loader::new(move |_, sr| {
            c.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            crate::sounds::load("Keys", sr)
        });
        l.request("file:/a.sfz", 48_000);
        l.request("file:/a.sfz", 48_000);
        let deadline = Instant::now() + Duration::from_secs(5);
        let got = loop {
            if let Some(x) = l.try_recv() {
                break x;
            }
            assert!(Instant::now() < deadline, "no result");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(got.sound, "file:/a.sfz");
        assert!(got.result.is_ok());
        std::thread::sleep(Duration::from_millis(60));
        assert!(l.try_recv().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Received, so it may be asked for again.
        l.request("file:/a.sfz", 48_000);
        assert_eq!(l.in_flight.len(), 1);
    }
}
