//! Writes voice takes to disk. The engine pushes microphone samples into a queue
//! while recording; a background thread empties it into a WAV file.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(10);

type Writer = hound::WavWriter<BufWriter<File>>;

/// A finished take on disk.
#[derive(Debug)]
pub struct Take {
    pub path: PathBuf,
    pub frames: u64,
}

enum Msg {
    Source(rtrb::Consumer<f32>),
    Start { path: PathBuf, sample_rate: u32 },
    Stop(mpsc::Sender<Result<Take, String>>),
    Cancel,
}

pub struct Recorder {
    tx: mpsc::Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn new(source: rtrb::Consumer<f32>) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("take-writer".into())
            .spawn(move || run(source, rx))
            .expect("spawn take writer");
        Self {
            tx,
            thread: Some(thread),
        }
    }

    /// Read samples from a new engine's queue (after the audio device restarted).
    pub fn set_source(&self, source: rtrb::Consumer<f32>) {
        let _ = self.tx.send(Msg::Source(source));
    }

    /// Start writing everything the engine sends into a new file at `path`.
    pub fn start(&self, path: PathBuf, sample_rate: u32) {
        let _ = self.tx.send(Msg::Start { path, sample_rate });
    }

    /// Finish the take: write what's left in the queue and close the file. Call this
    /// once the engine has reported the take finished, so nothing is still on its way.
    pub fn stop(&self) -> Result<Take, String> {
        let (reply, answer) = mpsc::channel();
        self.tx
            .send(Msg::Stop(reply))
            .map_err(|_| "the recorder stopped".to_string())?;
        answer
            .recv()
            .map_err(|_| "the recorder stopped".to_string())?
    }

    /// Throw the current take away and delete its file.
    pub fn cancel(&self) {
        let _ = self.tx.send(Msg::Cancel);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Closing the channel ends the thread.
        let (dead, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.tx, dead));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Current {
    path: PathBuf,
    writer: Option<Writer>,
    frames: u64,
    error: Option<String>,
}

fn create(path: &Path, sample_rate: u32) -> Result<Writer, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    hound::WavWriter::create(path, spec).map_err(|e| e.to_string())
}

fn drain(source: &mut rtrb::Consumer<f32>, take: &mut Option<Current>) {
    let n = source.slots();
    let Ok(chunk) = source.read_chunk(n) else {
        return;
    };
    if let Some(cur) = take
        && let Some(w) = &mut cur.writer
    {
        for s in chunk.into_iter() {
            if let Err(e) = w.write_sample(s) {
                cur.error = Some(format!("Could not save the recording: {e}"));
                cur.writer = None;
                return;
            }
            cur.frames += 1;
        }
    } else {
        // Not recording: throw away anything left from an earlier take.
        chunk.commit_all();
    }
}

fn run(mut source: rtrb::Consumer<f32>, rx: mpsc::Receiver<Msg>) {
    let mut take: Option<Current> = None;
    loop {
        let msg = rx.recv_timeout(POLL);
        drain(&mut source, &mut take);
        match msg {
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Source(s)) => source = s,
            Ok(Msg::Start { path, sample_rate }) => {
                if let Some(old) = take.take() {
                    discard(old);
                }
                let (writer, error) = match create(&path, sample_rate) {
                    Ok(w) => (Some(w), None),
                    Err(e) => (None, Some(format!("Could not start the recording: {e}"))),
                };
                take = Some(Current {
                    path,
                    writer,
                    frames: 0,
                    error,
                });
            }
            Ok(Msg::Stop(reply)) => {
                let result = match take.take() {
                    None => Err("Nothing was being recorded".to_string()),
                    Some(cur) => finish(cur),
                };
                let _ = reply.send(result);
            }
            Ok(Msg::Cancel) => {
                if let Some(old) = take.take() {
                    discard(old);
                }
            }
        }
    }
    if let Some(old) = take {
        discard(old);
    }
}

fn finish(cur: Current) -> Result<Take, String> {
    if let Some(e) = cur.error {
        let _ = std::fs::remove_file(&cur.path);
        return Err(e);
    }
    if let Some(w) = cur.writer {
        w.finalize()
            .map_err(|e| format!("Could not save the recording: {e}"))?;
    }
    Ok(Take {
        path: cur.path,
        frames: cur.frames,
    })
}

fn discard(cur: Current) {
    drop(cur.writer);
    let _ = std::fs::remove_file(&cur.path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_queued_samples_to_a_wav() {
        let dir = tempfile::tempdir().unwrap();
        let (mut tx, rx) = rtrb::RingBuffer::new(1024);
        let rec = Recorder::new(rx);
        let path = dir.path().join("audio").join("take-1.wav");
        // Samples queued before the take starts are not part of it.
        tx.push(9.0).unwrap();
        std::thread::sleep(POLL * 3);
        rec.start(path.clone(), 48_000);
        std::thread::sleep(POLL * 3);
        for i in 0..500 {
            tx.push(i as f32 / 1000.0).unwrap();
        }
        let take = rec.stop().unwrap();
        assert_eq!(take.frames, 500);
        let mut r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.spec().sample_rate, 48_000);
        let samples: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 500);
        assert_eq!(samples[250], 0.25);
    }

    #[test]
    fn cancel_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut tx, rx) = rtrb::RingBuffer::new(64);
        let rec = Recorder::new(rx);
        let path = dir.path().join("take.wav");
        rec.start(path.clone(), 48_000);
        tx.push(0.5).unwrap();
        rec.cancel();
        // A stop afterwards has nothing to finish.
        assert!(rec.stop().is_err());
        assert!(!path.exists());
    }

    #[test]
    fn unwritable_folder_reports_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let (_tx, rx) = rtrb::RingBuffer::new(64);
        let rec = Recorder::new(rx);
        rec.start(blocker.join("take.wav"), 48_000);
        assert!(rec.stop().unwrap_err().contains("Could not start"));
    }
}
