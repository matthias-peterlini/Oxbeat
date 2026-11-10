//! Downloading sound packs from the Oxbeat-beats repository, in the background.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::essentials;
use super::library::{self, PackManifest, is_safe_id, packs_dir};

pub const INDEX_URL: &str =
    "https://github.com/matthias-peterlini/Oxbeat-beats/releases/download/packs/index.json";
/// Refuse absurd downloads (and zip bombs) outright.
const MAX_PACK_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub author: String,
    pub size: u64,
    pub url: String,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Index {
    packs: Vec<IndexEntry>,
}

pub fn parse_index(text: &str) -> Result<Vec<IndexEntry>, String> {
    let index: Index = serde_json::from_str(text).map_err(|e| format!("bad index: {e}"))?;
    Ok(index
        .packs
        .into_iter()
        .filter(|p| is_safe_id(&p.id) && p.url.starts_with("https://"))
        .collect())
}

fn fetch_index() -> Result<Vec<IndexEntry>, String> {
    let mut resp = ureq::get(INDEX_URL).call().map_err(friendly)?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_index(&text)
}

fn friendly(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(404) => "The sound library isn't published yet.".into(),
        ureq::Error::StatusCode(c) => format!("The server answered {c}."),
        ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            "No internet connection.".into()
        }
        other => other.to_string(),
    }
}

/// Download `entry` into `dest_root/<id>`, reporting progress in permille.
pub fn install(entry: &IndexEntry, dest_root: &Path, report: &dyn Fn(u32)) -> Result<(), String> {
    if !is_safe_id(&entry.id) {
        return Err("bad pack id".into());
    }
    if entry.size > MAX_PACK_BYTES {
        return Err("pack is too large".into());
    }
    std::fs::create_dir_all(dest_root).map_err(|e| e.to_string())?;
    let zip_path = dest_root.join(format!(".download-{}.zip", entry.id));
    let result = (|| {
        let mut resp = ureq::get(&entry.url).call().map_err(friendly)?;
        let total = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(entry.size)
            .max(1);
        let mut reader = resp.body_mut().with_config().limit(MAX_PACK_BYTES).reader();
        let mut file = std::fs::File::create(&zip_path).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            report((done * 900 / total).min(900) as u32);
        }
        drop(file);
        let digest: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !digest.eq_ignore_ascii_case(entry.sha256.trim()) {
            return Err("the download was damaged (checksum mismatch)".to_string());
        }
        extract_pack(&zip_path, dest_root, &entry.id)?;
        report(1000);
        Ok(())
    })();
    let _ = std::fs::remove_file(&zip_path);
    result
}

/// Unzip into `dest_root/<id>`, replacing any older version. The zip must contain
/// `pack.json` at its root or inside a single top-level folder.
pub fn extract_pack(zip_path: &Path, dest_root: &Path, id: &str) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip file ({e})"))?;
    let tmp = dest_root.join(format!(".unpack-{id}"));
    let _ = std::fs::remove_dir_all(&tmp);
    let result = (|| {
        let mut written = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            // enclosed_name rejects absolute paths and `..` escapes.
            let Some(rel) = f.enclosed_name() else {
                return Err("the pack contains unsafe file names".to_string());
            };
            let out = tmp.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            written += std::io::copy(&mut (&mut f).take(MAX_PACK_BYTES - written), &mut w)
                .map_err(|e| e.to_string())?;
            if written >= MAX_PACK_BYTES {
                return Err("pack is too large".to_string());
            }
        }
        let root = pack_root(&tmp).ok_or("pack.json is missing")?;
        let manifest = library::read_manifest(&root)?;
        if manifest.id != id {
            return Err(format!("pack id mismatch ({} != {id})", manifest.id));
        }
        let dest = dest_root.join(id);
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&root, &dest).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn pack_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("pack.json").is_file() {
        return Some(dir.to_path_buf());
    }
    let mut subdirs = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir());
    let only = subdirs.next()?;
    if subdirs.next().is_some() {
        return None;
    }
    only.path().join("pack.json").is_file().then(|| only.path())
}

/// Delete what an interrupted download left in `dir` (`.download-*.zip` files and
/// `.unpack-*` folders). Call before any download starts.
pub fn sweep_temp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        if name.starts_with(".download-") && name.ends_with(".zip") {
            let _ = std::fs::remove_file(&p);
        } else if name.starts_with(".unpack-") {
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
        }
    }
}

pub fn remove(id: &str) -> Result<(), String> {
    if !is_safe_id(id) {
        return Err("bad pack id".into());
    }
    std::fs::remove_dir_all(packs_dir().join(id)).map_err(|e| e.to_string())
}

pub enum IndexState {
    NotLoaded,
    Loading,
    Ready(Vec<IndexEntry>),
    Failed(String),
}

/// What the "instrument library" strip should show.
#[derive(Debug, Clone, PartialEq)]
pub enum EssentialsStatus {
    Hidden,
    Downloading { permille: u32, size: u64 },
    Failed(String),
}

enum Msg {
    Index(Result<Vec<IndexEntry>, String>),
    Installed(String, Result<(), String>),
}

/// Runs index fetches and installs on background threads; poll from the UI.
pub struct PackManager {
    pub index: IndexState,
    pub installed: Vec<PackManifest>,
    /// Pack id -> progress in permille.
    pub downloads: HashMap<String, Arc<AtomicU32>>,
    pub errors: Vec<String>,
    /// Ids of packs installed since the app started. Callers drain it.
    pub just_installed: Vec<String>,
    essentials: essentials::State,
    /// The auto-install was started this session; don't start it again.
    essentials_tried: bool,
    essentials_error: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl PackManager {
    pub fn new(repaint: Arc<dyn Fn() + Send + Sync>) -> PackManager {
        let (tx, rx) = channel();
        // Nothing is downloading yet, so anything temporary is from a quit mid-download.
        sweep_temp(&packs_dir());
        PackManager {
            index: IndexState::NotLoaded,
            installed: library::installed_packs(),
            downloads: HashMap::new(),
            errors: Vec::new(),
            just_installed: Vec::new(),
            essentials: essentials::load(&packs_dir()),
            essentials_tried: false,
            essentials_error: None,
            tx,
            rx,
            repaint,
        }
    }

    pub fn is_installed(&self, id: &str) -> Option<&PackManifest> {
        self.installed.iter().find(|p| p.id == id)
    }

    pub fn refresh_installed(&mut self) {
        self.installed = library::installed_packs();
    }

    /// Fetch the index if we haven't (or the last try failed).
    pub fn load_index(&mut self) {
        if matches!(self.index, IndexState::Loading | IndexState::Ready(_)) {
            return;
        }
        self.index = IndexState::Loading;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Index(fetch_index()));
            repaint();
        });
    }

    pub fn retry_index(&mut self) {
        self.index = IndexState::NotLoaded;
        self.load_index();
    }

    pub fn install(&mut self, entry: IndexEntry) {
        if self.downloads.contains_key(&entry.id) {
            return;
        }
        if entry.id == essentials::ID {
            self.set_removed_by_user(false);
            self.essentials_error = None;
        }
        let progress = Arc::new(AtomicU32::new(0));
        self.downloads.insert(entry.id.clone(), progress.clone());
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let report = |p: u32| {
                progress.store(p, Ordering::Relaxed);
                repaint();
            };
            let r = install(&entry, &packs_dir(), &report);
            let _ = tx.send(Msg::Installed(entry.id.clone(), r));
            repaint();
        });
    }

    pub fn uninstall(&mut self, id: &str) {
        if let Err(e) = remove(id) {
            self.errors.push(format!("Could not remove the pack: {e}"));
        } else if id == essentials::ID {
            self.set_removed_by_user(true);
        }
        self.refresh_installed();
    }

    fn set_removed_by_user(&mut self, removed: bool) {
        if self.essentials.removed_by_user != removed {
            self.essentials.removed_by_user = removed;
            essentials::save(&packs_dir(), &self.essentials);
        }
    }

    fn essentials_wanted(&self) -> bool {
        essentials::should_auto_install(
            self.is_installed(essentials::ID).is_some(),
            self.downloads.contains_key(essentials::ID),
            &self.essentials,
        )
    }

    /// Call every frame. Downloads the essentials pack once per launch when it is
    /// missing and the user hasn't removed it.
    pub fn ensure_essentials(&mut self) {
        if self.essentials_tried || !self.essentials_wanted() {
            return;
        }
        match &self.index {
            IndexState::NotLoaded => self.load_index(),
            IndexState::Loading | IndexState::Failed(_) => {}
            IndexState::Ready(list) => {
                // Not published yet: stay quiet.
                if let Some(entry) = list.iter().find(|e| e.id == essentials::ID).cloned() {
                    self.essentials_tried = true;
                    self.install(entry);
                }
            }
        }
    }

    /// The user asked for the instruments ("Install instruments"): allow and start
    /// the download even after a removal or a failure.
    pub fn request_essentials(&mut self) {
        self.set_removed_by_user(false);
        self.essentials_tried = false;
        self.essentials_error = None;
        if matches!(self.index, IndexState::Failed(_)) {
            self.retry_index();
        }
    }

    /// True while Essentials is not installed, so the Library tab can offer it.
    pub fn essentials_missing(&self) -> bool {
        self.is_installed(essentials::ID).is_none()
    }

    pub fn essentials_status(&self) -> EssentialsStatus {
        if let Some(p) = self.downloads.get(essentials::ID) {
            let size = match &self.index {
                IndexState::Ready(list) => list
                    .iter()
                    .find(|e| e.id == essentials::ID)
                    .map_or(0, |e| e.size),
                _ => 0,
            };
            return EssentialsStatus::Downloading {
                permille: p.load(Ordering::Relaxed),
                size,
            };
        }
        if let Some(e) = &self.essentials_error {
            return EssentialsStatus::Failed(e.clone());
        }
        if self.essentials_wanted()
            && let IndexState::Failed(e) = &self.index
        {
            return EssentialsStatus::Failed(e.clone());
        }
        EssentialsStatus::Hidden
    }

    /// Apply finished background work. Returns true if the installed list changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Index(Ok(list)) => self.index = IndexState::Ready(list),
                Msg::Index(Err(e)) => self.index = IndexState::Failed(e),
                Msg::Installed(id, r) => {
                    self.downloads.remove(&id);
                    match r {
                        Ok(()) => self.just_installed.push(id),
                        // The strip reports this one; no modal for a background download.
                        Err(e) if id == essentials::ID => self.essentials_error = Some(e),
                        Err(e) => self.errors.push(format!("Could not install the pack: {e}")),
                    }
                    changed = true;
                }
            }
        }
        if changed {
            self.refresh_installed();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip(path: &Path, files: &[(&str, &str)]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, body) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    const MANIFEST: &str = r#"{"id":"demo","name":"Demo","sounds":[]}"#;

    #[test]
    fn parses_index_and_drops_unsafe_entries() {
        let list = parse_index(
            r#"{"packs":[
              {"id":"ok","name":"OK","size":1,"url":"https://x/ok.zip","sha256":"ab"},
              {"id":"../bad","name":"Bad","size":1,"url":"https://x/b.zip","sha256":"ab"},
              {"id":"http","name":"Plain","size":1,"url":"http://x/b.zip","sha256":"ab"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "ok");
    }

    #[test]
    fn extracts_pack_with_top_folder() {
        let dir = tempfile::tempdir().unwrap();
        let z = dir.path().join("p.zip");
        make_zip(&z, &[("Demo/pack.json", MANIFEST), ("Demo/a.wav", "x")]);
        extract_pack(&z, dir.path(), "demo").unwrap();
        assert!(dir.path().join("demo/a.wav").is_file());
        assert!(!dir.path().join(".unpack-demo").exists());
    }

    #[test]
    fn rejects_zip_slip_and_wrong_id() {
        let dir = tempfile::tempdir().unwrap();
        let z = dir.path().join("p.zip");
        make_zip(&z, &[("pack.json", MANIFEST), ("../evil.txt", "x")]);
        assert!(extract_pack(&z, dir.path(), "demo").is_err());
        assert!(!dir.path().join("../evil.txt").exists());

        make_zip(&z, &[("pack.json", MANIFEST)]);
        assert!(extract_pack(&z, dir.path(), "other").is_err());
        assert!(!dir.path().join("other").exists());
    }

    #[test]
    fn sweeps_leftover_download_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join(".download-essentials.zip"), "partial").unwrap();
        std::fs::create_dir_all(d.join(".unpack-essentials/samples")).unwrap();
        std::fs::write(d.join(".unpack-essentials/samples/a.flac"), "x").unwrap();
        std::fs::create_dir_all(d.join("demo")).unwrap();
        std::fs::write(d.join("demo/pack.json"), MANIFEST).unwrap();
        std::fs::write(d.join("essentials.json"), "{}").unwrap();
        sweep_temp(d);
        assert!(!d.join(".download-essentials.zip").exists());
        assert!(!d.join(".unpack-essentials").exists());
        assert!(d.join("demo/pack.json").is_file());
        assert!(d.join("essentials.json").is_file());
        sweep_temp(&d.join("missing")); // no packs dir yet: nothing to do
    }

    #[test]
    fn reinstall_replaces_old_version() {
        let dir = tempfile::tempdir().unwrap();
        let z = dir.path().join("p.zip");
        make_zip(&z, &[("pack.json", MANIFEST), ("old.wav", "x")]);
        extract_pack(&z, dir.path(), "demo").unwrap();
        make_zip(&z, &[("pack.json", MANIFEST), ("new.wav", "x")]);
        extract_pack(&z, dir.path(), "demo").unwrap();
        assert!(dir.path().join("demo/new.wav").is_file());
        assert!(!dir.path().join("demo/old.wav").exists());
    }
}
