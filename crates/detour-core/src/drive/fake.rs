//! An in-memory drive, for tests of the mirror and of vaults kept in a drive.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::mirror::{Item, Listing, Storage};
use crate::Result;

#[derive(Default)]
struct Inner {
    /// Path -> (id, version, contents).
    files: BTreeMap<String, (String, u64, Vec<u8>)>,
    /// Folder path -> id.
    folders: BTreeMap<String, String>,
    next: u64,
}

impl Inner {
    fn id(&mut self) -> String {
        self.next += 1;
        format!("f{}", self.next)
    }

    fn path_of_folder(&self, id: Option<&str>) -> String {
        match id {
            None => String::new(),
            Some(id) => self.folders.iter().find(|(_, v)| v.as_str() == id).map(|(k, _)| k.clone()).unwrap_or_default(),
        }
    }
}

/// A drive shared by the devices of a test.
#[derive(Clone, Default)]
pub struct FakeDrive(Arc<Mutex<Inner>>);

impl FakeDrive {
    pub fn new() -> Self {
        Self::default()
    }

    /// The vault's folder in it.
    pub fn folder(&self) -> FakeFolder {
        FakeFolder(self.0.clone())
    }

    pub fn has(&self, path: &str) -> bool {
        self.0.lock().unwrap().files.contains_key(path)
    }

    pub fn read(&self, path: &str) -> Option<String> {
        self.0.lock().unwrap().files.get(path).map(|f| String::from_utf8_lossy(&f.2).to_string())
    }

    pub fn remove(&self, path: &str) {
        self.0.lock().unwrap().files.remove(path);
    }

    pub fn paths(&self) -> Vec<String> {
        self.0.lock().unwrap().files.keys().cloned().collect()
    }
}

pub struct FakeFolder(Arc<Mutex<Inner>>);

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

impl Storage for FakeFolder {
    fn list(&self) -> Result<Listing> {
        let d = self.0.lock().unwrap();
        Ok(Listing {
            files: d.files.iter().map(|(p, (id, v, _))| (p.clone(), Item { id: id.clone(), version: v.to_string() })).collect(),
            folders: d.folders.clone(),
        })
    }

    fn download(&self, item: &Item) -> Result<Vec<u8>> {
        let d = self.0.lock().unwrap();
        d.files.values().find(|f| f.0 == item.id).map(|f| f.2.clone()).ok_or_else(|| crate::Error::DriveNoAccess { provider: "fake".into() })
    }

    fn create(&self, parent: Option<&str>, name: &str, data: &[u8]) -> Result<Option<Item>> {
        let mut d = self.0.lock().unwrap();
        let path = join(&d.path_of_folder(parent), name);
        if d.files.contains_key(&path) {
            return Ok(None);
        }
        let id = d.id();
        d.files.insert(path, (id.clone(), 1, data.to_vec()));
        Ok(Some(Item { id, version: "1".into() }))
    }

    fn replace(&self, item: &Item, data: &[u8]) -> Result<Option<Item>> {
        let mut d = self.0.lock().unwrap();
        let Some(f) = d.files.values_mut().find(|f| f.0 == item.id) else { return Ok(None) };
        if f.1.to_string() != item.version {
            return Ok(None);
        }
        f.1 += 1;
        f.2 = data.to_vec();
        Ok(Some(Item { id: f.0.clone(), version: f.1.to_string() }))
    }

    fn delete(&self, item: &Item) -> Result<()> {
        let mut d = self.0.lock().unwrap();
        d.files.retain(|_, f| !(f.0 == item.id && f.1.to_string() == item.version));
        Ok(())
    }

    fn folder(&self, parent: Option<&str>, name: &str) -> Result<String> {
        let mut d = self.0.lock().unwrap();
        let path = join(&d.path_of_folder(parent), name);
        if let Some(id) = d.folders.get(&path) {
            return Ok(id.clone());
        }
        let id = d.id();
        d.folders.insert(path, id.clone());
        Ok(id)
    }
}
