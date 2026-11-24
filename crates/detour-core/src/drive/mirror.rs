//! A vault's working copy mirrored with a folder in a cloud drive.
//!
//! The rules are those of the Git transport: a file this device has is never replaced or
//! deleted by what comes in (another version of it is kept beside it, and reported as a
//! second version of that entry); a file the drive lost is sent again; what this device
//! changed or deleted itself goes out, unless the drive's copy changed meanwhile.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// A file in the drive: its id there and a version that changes with its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub version: String,
}

/// Everything under the vault's folder.
#[derive(Debug, Default, Clone)]
pub struct Listing {
    /// Path (`ops/dev/00000001.json`) -> file.
    pub files: BTreeMap<String, Item>,
    /// Path (`ops/dev`) -> folder id. The root is not listed.
    pub folders: BTreeMap<String, String>,
}

/// Files at the same time while pulling.
const DOWNLOADS: usize = 8;

/// Pulls of at least this many files report how far they are (opening a team or a vault on a
/// new device; an everyday sync brings a few).
const REPORTED: usize = 10;

type Progress = std::sync::Arc<dyn Fn(usize, usize) + Send + Sync>;
static PROGRESS: std::sync::Mutex<Option<Progress>> = std::sync::Mutex::new(None);

/// Called with (files done, files in all) while a long pull downloads. Replaces the one before.
pub fn on_progress(f: impl Fn(usize, usize) + Send + Sync + 'static) {
    *PROGRESS.lock().unwrap() = Some(std::sync::Arc::new(f));
}

/// Downloads the items, `DOWNLOADS` at a time, in their order. The first failure is returned.
fn download_all(storage: &dyn Storage, items: &[(&String, &Item)]) -> Result<Vec<Vec<u8>>> {
    if items.len() < 2 {
        return items.iter().map(|(_, i)| storage.download(i)).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let report = PROGRESS.lock().unwrap().clone().filter(|_| items.len() >= REPORTED);
    if let Some(f) = &report {
        f(0, items.len());
    }
    let failed = std::sync::atomic::AtomicBool::new(false);
    let slots: Vec<std::sync::Mutex<Option<Result<Vec<u8>>>>> = items.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..DOWNLOADS.min(items.len()) {
            s.spawn(|| {
                use std::sync::atomic::Ordering::Relaxed;
                while !failed.load(Relaxed) {
                    let n = next.fetch_add(1, Relaxed);
                    let Some((_, item)) = items.get(n) else { break };
                    let r = storage.download(item);
                    if r.is_err() {
                        failed.store(true, Relaxed);
                    }
                    let d = done.fetch_add(1, Relaxed) + 1;
                    if let Some(f) = &report {
                        f(d, items.len());
                    }
                    *slots[n].lock().unwrap() = Some(r);
                }
            });
        }
    });
    let mut got: Vec<Option<Result<Vec<u8>>>> = slots.into_iter().map(|m| m.into_inner().unwrap()).collect();
    // Some were not fetched only when one failed: that failure is the answer.
    if let Some(i) = got.iter().position(|r| matches!(r, Some(Err(_)))) {
        return Err(got.swap_remove(i).unwrap().unwrap_err());
    }
    Ok(got.into_iter().map(|r| r.expect("each fetched").expect("no failure")).collect())
}

/// One drive's API, on the vault's folder. Blocking: called off the async threads.
pub trait Storage: Send + Sync {
    fn list(&self) -> Result<Listing>;
    fn download(&self, item: &Item) -> Result<Vec<u8>>;
    /// Creates a file in a folder (`None`: the vault's folder). `Ok(None)`: one with that
    /// name is there already.
    fn create(&self, parent: Option<&str>, name: &str, data: &[u8]) -> Result<Option<Item>>;
    /// Replaces a file's contents if it is still at `item.version`. `Ok(None)`: it changed.
    fn replace(&self, item: &Item, data: &[u8]) -> Result<Option<Item>>;
    /// Deletes a file if it is still at `item.version`; gone or changed is fine.
    fn delete(&self, item: &Item) -> Result<()>;
    /// A folder in a folder (`None`: the vault's folder), made if missing. Returns its id.
    fn folder(&self, parent: Option<&str>, name: &str) -> Result<String>;
}

/// What was last seen in the drive, and what of it this device holds the same.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// The drive's files as last listed.
    #[serde(default)]
    remote: BTreeMap<String, Item>,
    /// For each of them: the hash of the local file that stands for it here. Ours differs
    /// when both changed it (the drive's version is kept beside ours) or we changed it since.
    #[serde(default)]
    synced: BTreeMap<String, String>,
    #[serde(default)]
    folders: BTreeMap<String, String>,
    /// Paths where both sides have their own version: ours (this hash) still has to go to
    /// the drive, beside theirs.
    #[serde(default)]
    beside: BTreeMap<String, String>,
}

/// The result of mirroring with the drive.
#[derive(Debug, Default)]
pub struct Synced {
    pub pulled: bool,
    pub pushed: bool,
}

#[derive(Clone)]
pub struct Mirror {
    dir: PathBuf,
}

fn hash(data: &[u8]) -> String {
    let h = Sha256::digest(data);
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn bad(what: &str) -> Error {
    Error::Internal { message: format!("drive copy: {what}") }
}

/// A second version of `path` that the drive holds, kept beside ours.
pub fn conflict_name(path: &str, data_hash: &str) -> String {
    let short = &data_hash[..12.min(data_hash.len())];
    let (dir, file) = path.rsplit_once('/').map_or(("", path), |(d, f)| (d, f));
    let renamed = match file.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.drive-conflict-{short}.{ext}"),
        None => format!("{file}.drive-conflict-{short}"),
    };
    if dir.is_empty() { renamed } else { format!("{dir}/{renamed}") }
}

/// Writes through a temp file, so a reader never sees half a file.
fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Mirror {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }

    /// The working copy: what a vault reads and writes.
    pub fn files(&self) -> PathBuf {
        self.dir.join("files")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("drive.json")
    }

    fn state(&self) -> Result<State> {
        match std::fs::read(self.state_path()) {
            Ok(b) => serde_json::from_slice(&b).map_err(|_| bad("drive.json")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save_state(&self, st: &State) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_file(&self.state_path(), &serde_json::to_vec_pretty(st)?)
    }

    /// Every file of the working copy with its hash. Temp files are skipped.
    fn local(&self) -> Result<BTreeMap<String, String>> {
        fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, String>) -> Result<()> {
            for e in std::fs::read_dir(dir)? {
                let e = e?;
                let name = e.file_name().to_string_lossy().to_string();
                let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                if e.file_type()?.is_dir() {
                    walk(&e.path(), &rel, out)?;
                } else if !name.ends_with(".tmp") {
                    out.insert(rel, hash(&std::fs::read(e.path())?));
                }
            }
            Ok(())
        }
        let mut out = BTreeMap::new();
        if self.files().is_dir() {
            walk(&self.files(), "", &mut out)?;
        }
        Ok(out)
    }

    /// Local changes the drive doesn't have yet.
    pub fn pending(&self) -> Result<bool> {
        let st = self.state()?;
        let local = self.local()?;
        Ok(!st.beside.is_empty() || local.iter().any(|(p, h)| st.synced.get(p) != Some(h)) || st.synced.keys().any(|p| !local.contains_key(p)))
    }

    /// Is the drive's folder empty? Checked before making a vault there.
    pub fn is_empty(storage: &dyn Storage) -> Result<bool> {
        Ok(storage.list()?.files.is_empty())
    }

    /// Brings the drive's changes into the working copy.
    pub fn pull(&self, storage: &dyn Storage) -> Result<Synced> {
        let mut st = self.state()?;
        let listing = storage.list()?;
        let mut synced = Synced::default();
        let local = self.local()?;
        // What changed there, fetched several at a time: one request per file, and a team or a
        // vault opened on a new device is hundreds of them.
        let wanted: Vec<(&String, &Item)> = listing.files.iter().filter(|(p, i)| st.remote.get(*p) != Some(*i)).collect();
        let fetched = download_all(storage, &wanted)?;
        for ((path, item), data) in wanted.into_iter().zip(fetched) {
            let h = hash(&data);
            match local.get(path) {
                // The same here already.
                Some(ours) if *ours == h => {}
                // Ours is something else: theirs goes beside it here, ours beside theirs there
                // (on the next push), and ours stays what stands for the path here.
                Some(ours) => {
                    let beside = conflict_name(path, &h);
                    if local.get(&beside) != Some(&h) {
                        write_file(&self.files().join(&beside), &data)?;
                        synced.pulled = true;
                    }
                    st.synced.insert(beside, h);
                    st.beside.insert(path.clone(), ours.clone());
                    st.synced.insert(path.clone(), ours.clone());
                    st.remote.insert(path.clone(), item.clone());
                    continue;
                }
                None => {
                    write_file(&self.files().join(path), &data)?;
                    synced.pulled = true;
                }
            }
            st.synced.insert(path.clone(), h);
            st.remote.insert(path.clone(), item.clone());
        }
        // Gone from the drive: forgotten here, so that push sends ours again.
        let gone: Vec<String> = st.remote.keys().filter(|p| !listing.files.contains_key(*p)).cloned().collect();
        for p in gone {
            st.remote.remove(&p);
            st.synced.remove(&p);
        }
        st.folders = listing.folders;
        self.save_state(&st)?;
        Ok(synced)
    }

    /// Sends what changed here: new files, changed ones, deleted ones.
    pub fn push(&self, storage: &dyn Storage) -> Result<Synced> {
        let mut st = self.state()?;
        let local = self.local()?;
        let mut synced = Synced::default();
        // Our versions of paths the drive has another version of.
        for (path, h) in std::mem::take(&mut st.beside) {
            if local.get(&path) != Some(&h) {
                continue;
            }
            let name = conflict_name(&path, &h);
            if !st.remote.contains_key(&name) {
                let data = std::fs::read(self.files().join(&path))?;
                if let Some(item) = self.create_at(storage, &mut st, &name, &data)? {
                    st.remote.insert(name, item);
                }
                synced.pushed = true;
            }
        }
        for (path, h) in &local {
            if st.synced.get(path) == Some(h) {
                continue;
            }
            let data = std::fs::read(self.files().join(path))?;
            match st.remote.get(path).cloned() {
                // Changed here since: replace theirs, if it is still the one we saw.
                Some(item) => match storage.replace(&item, &data)? {
                    Some(new) => {
                        st.remote.insert(path.clone(), new);
                        st.synced.insert(path.clone(), h.clone());
                        synced.pushed = true;
                    }
                    // It changed there too: ours goes beside it; the next pull brings theirs.
                    None => {
                        self.beside_there(storage, &mut st, path, h, &data)?;
                        st.synced.insert(path.clone(), h.clone());
                        synced.pushed = true;
                    }
                },
                None => {
                    match self.create_at(storage, &mut st, path, &data)? {
                        Some(item) => {
                            st.remote.insert(path.clone(), item);
                        }
                        // Another device wrote that path first: ours goes beside it.
                        None => self.beside_there(storage, &mut st, path, h, &data)?,
                    }
                    st.synced.insert(path.clone(), h.clone());
                    synced.pushed = true;
                }
            }
        }
        // Deleted here: deleted there too, unless it changed there meanwhile.
        let deleted: Vec<String> = st.synced.keys().filter(|p| !local.contains_key(*p)).cloned().collect();
        for p in deleted {
            if let Some(item) = st.remote.remove(&p) {
                storage.delete(&item)?;
                synced.pushed = true;
            }
            st.synced.remove(&p);
        }
        self.save_state(&st)?;
        Ok(synced)
    }

    /// Pull, then push what is pending.
    pub fn sync(&self, storage: &dyn Storage) -> Result<Synced> {
        let a = self.pull(storage)?;
        let b = self.push(storage)?;
        Ok(Synced { pulled: a.pulled, pushed: b.pushed })
    }

    /// Creates `path` in the drive, making its folders first. `None`: it exists already.
    fn create_at(&self, storage: &dyn Storage, st: &mut State, path: &str, data: &[u8]) -> Result<Option<Item>> {
        let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
        let parent = if dir.is_empty() { None } else { Some(self.folder_id(storage, st, dir)?) };
        storage.create(parent.as_deref(), name, data)
    }

    /// Our version of `path` into the drive beside theirs. Only the drive has it under that
    /// name: here it is still `path`.
    fn beside_there(&self, storage: &dyn Storage, st: &mut State, path: &str, h: &str, data: &[u8]) -> Result<()> {
        let name = conflict_name(path, h);
        if !st.remote.contains_key(&name)
            && let Some(item) = self.create_at(storage, st, &name, data)?
        {
            st.remote.insert(name, item);
        }
        Ok(())
    }

    fn folder_id(&self, storage: &dyn Storage, st: &mut State, dir: &str) -> Result<String> {
        if let Some(id) = st.folders.get(dir) {
            return Ok(id.clone());
        }
        let (parent_dir, name) = dir.rsplit_once('/').map_or(("", dir), |(p, n)| (p, n));
        let parent = if parent_dir.is_empty() { None } else { Some(self.folder_id(storage, st, parent_dir)?) };
        let id = storage.folder(parent.as_deref(), name)?;
        st.folders.insert(dir.to_string(), id.clone());
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::fake::FakeDrive;

    fn mirror() -> Mirror {
        let dir = std::env::temp_dir().join(format!("detour-drive-{}", crate::new_id()));
        Mirror::new(dir)
    }

    fn put(m: &Mirror, path: &str, data: &str) {
        write_file(&m.files().join(path), data.as_bytes()).unwrap();
    }

    fn read(m: &Mirror, path: &str) -> Option<String> {
        std::fs::read_to_string(m.files().join(path)).ok()
    }

    fn names(m: &Mirror) -> Vec<String> {
        m.local().unwrap().keys().cloned().collect()
    }

    #[test]
    fn two_devices_converge() {
        let drive = FakeDrive::new();
        let (a, b) = (mirror(), mirror());
        put(&a, "vault.json", "v");
        put(&a, "ops/a/00000001.json", "a1");
        assert!(a.pending().unwrap());
        assert!(a.sync(&drive.folder()).unwrap().pushed);
        assert!(!a.pending().unwrap());

        assert!(b.sync(&drive.folder()).unwrap().pulled);
        assert_eq!(read(&b, "ops/a/00000001.json").as_deref(), Some("a1"));
        put(&b, "ops/b/00000001.json", "b1");
        b.sync(&drive.folder()).unwrap();

        a.sync(&drive.folder()).unwrap();
        assert_eq!(names(&a), names(&b));
        assert_eq!(names(&a), vec!["ops/a/00000001.json", "ops/b/00000001.json", "vault.json"]);
        assert!(!a.pending().unwrap() && !b.pending().unwrap());
    }

    #[test]
    fn same_path_from_two_devices_keeps_both_versions() {
        let drive = FakeDrive::new();
        let (a, b) = (mirror(), mirror());
        put(&a, "pair/x.age", "from a");
        put(&b, "pair/x.age", "from b");
        a.sync(&drive.folder()).unwrap();
        b.sync(&drive.folder()).unwrap();
        a.sync(&drive.folder()).unwrap();
        // Each keeps its own at the path, the other's beside it; no file is overwritten.
        assert_eq!(read(&a, "pair/x.age").as_deref(), Some("from a"));
        assert_eq!(read(&b, "pair/x.age").as_deref(), Some("from b"));
        let both = |m: &Mirror| -> Vec<String> { m.local().unwrap().keys().filter_map(|p| read(m, p)).collect::<std::collections::BTreeSet<_>>().into_iter().collect() };
        assert_eq!(both(&a), vec!["from a", "from b"]);
        assert_eq!(both(&b), vec!["from a", "from b"]);
        // Nothing keeps going back and forth.
        assert!(!a.sync(&drive.folder()).unwrap().pushed);
        assert!(!b.sync(&drive.folder()).unwrap().pushed);
    }

    #[test]
    fn deletes_and_changes_go_out_and_lost_files_come_back() {
        let drive = FakeDrive::new();
        let (a, b) = (mirror(), mirror());
        put(&a, "pair/offer.age", "1");
        put(&a, "ops/a/00000001.json", "a1");
        a.sync(&drive.folder()).unwrap();
        b.sync(&drive.folder()).unwrap();

        // A changes the offer, then deletes it: B follows.
        put(&a, "pair/offer.age", "2");
        a.sync(&drive.folder()).unwrap();
        b.sync(&drive.folder()).unwrap();
        // B has the old one: it is kept, the new one beside it.
        assert_eq!(read(&b, "pair/offer.age").as_deref(), Some("1"));
        std::fs::remove_file(a.files().join("pair/offer.age")).unwrap();
        a.sync(&drive.folder()).unwrap();
        assert!(!drive.has("pair/offer.age"));

        // The drive loses a file: whoever has it sends it again.
        drive.remove("ops/a/00000001.json");
        b.sync(&drive.folder()).unwrap();
        assert!(drive.has("ops/a/00000001.json"));
    }

    #[test]
    fn a_change_there_is_not_overwritten_by_ours() {
        let drive = FakeDrive::new();
        let (a, b) = (mirror(), mirror());
        put(&a, "pair/p.age", "base");
        a.sync(&drive.folder()).unwrap();
        b.sync(&drive.folder()).unwrap();
        put(&a, "pair/p.age", "a's");
        put(&b, "pair/p.age", "b's");
        a.sync(&drive.folder()).unwrap();
        // B's replace finds the drive's copy changed: B's goes beside it.
        b.sync(&drive.folder()).unwrap();
        assert_eq!(drive.read("pair/p.age").as_deref(), Some("a's"));
        assert!(drive.paths().iter().any(|p| p.contains(".drive-conflict-")));
        a.sync(&drive.folder()).unwrap();
        b.sync(&drive.folder()).unwrap();
        let versions = |m: &Mirror| m.local().unwrap().keys().filter_map(|p| read(m, p)).filter(|s| s != "base").collect::<std::collections::BTreeSet<_>>();
        assert_eq!(versions(&a), versions(&b));
    }

    #[test]
    fn conflict_names_keep_the_extension() {
        assert_eq!(conflict_name("ops/a/00000001.json", "abcdef0123456789"), "ops/a/00000001.drive-conflict-abcdef012345.json");
        assert_eq!(conflict_name("README", "abcdef0123456789"), "README.drive-conflict-abcdef012345");
    }
}
