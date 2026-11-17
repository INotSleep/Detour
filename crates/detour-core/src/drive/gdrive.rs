//! Google Drive through the Drive API v3 (with the `drive` scope, so folders others share work).

use std::collections::BTreeMap;

use serde::Deserialize;

use super::http::{AGENT, SendJson, bearer, enc, reply};
use super::mirror::{Item, Listing, Storage, conflict_name};
use super::{DriveFolder, DriveListing, DriveRemote, Provider};
use crate::{Error, Result};

const P: Provider = Provider::GoogleDrive;
const FOLDER: &str = "application/vnd.google-apps.folder";

fn api() -> String {
    std::env::var("DETOUR_GDRIVE_API").unwrap_or_else(|_| "https://www.googleapis.com".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    id: String,
    name: String,
    #[serde(default)]
    mime_type: String,
    /// Grows with every change of the file.
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    files: Vec<File>,
    #[serde(default)]
    next_page_token: Option<String>,
}

impl File {
    fn is_folder(&self) -> bool {
        self.mime_type == FOLDER
    }

    fn item(&self) -> Item {
        Item { id: self.id.clone(), version: self.version.clone().unwrap_or_default() }
    }
}

/// A Drive query's string literal.
fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn query(token: &str, q: &str, extra: &str) -> Result<Vec<File>> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut url = format!(
            "{}/drive/v3/files?q={}&fields=nextPageToken,files(id,name,mimeType,version)&pageSize=1000&supportsAllDrives=true&includeItemsFromAllDrives=true{extra}",
            api(),
            enc(q)
        );
        if let Some(t) = &page {
            url.push_str(&format!("&pageToken={}", enc(t)));
        }
        let p: Page = reply(P, AGENT.get(&url).header("Authorization", &bearer(token)).call())?.expect(P)?;
        out.extend(p.files);
        match p.next_page_token {
            Some(t) => page = Some(t),
            None => return Ok(out),
        }
    }
}

fn children(token: &str, folder: &str) -> Result<Vec<File>> {
    query(token, &format!("{} in parents and trashed=false", quoted(folder)), "")
}

fn file(token: &str, id: &str) -> Result<File> {
    let url = format!("{}/drive/v3/files/{}?fields=id,name,mimeType,version&supportsAllDrives=true", api(), enc(id));
    reply(P, AGENT.get(&url).header("Authorization", &bearer(token)).call())?.expect(P)
}

/// The vault's folder.
pub struct Folder {
    token: String,
    root: String,
}

impl Folder {
    pub fn new(token: &str, remote: &DriveRemote) -> Result<Folder> {
        Ok(Folder { token: token.to_string(), root: remote.id.clone() })
    }

    fn named(&self, parent: &str, name: &str) -> Result<Vec<File>> {
        query(&self.token, &format!("name={} and {} in parents and trashed=false", quoted(name), quoted(parent)), "")
    }
}

/// A multipart/related body: the file's metadata, then its contents.
fn multipart(meta: &serde_json::Value, data: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("detour{}", crate::new_id());
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/related; boundary={boundary}"), body)
}

impl Storage for Folder {
    fn list(&self) -> Result<Listing> {
        let mut out = Listing::default();
        let mut todo = vec![(String::new(), self.root.clone())];
        while let Some((dir, id)) = todo.pop() {
            // Drive lets two files share a name: the later ones count as other versions.
            let mut seen: BTreeMap<String, ()> = BTreeMap::new();
            for f in children(&self.token, &id)? {
                let path = if dir.is_empty() { f.name.clone() } else { format!("{dir}/{}", f.name) };
                if f.is_folder() {
                    out.folders.entry(path.clone()).or_insert_with(|| f.id.clone());
                    todo.push((path, f.id));
                } else if seen.insert(path.clone(), ()).is_some() {
                    out.files.insert(conflict_name(&path, &f.id), f.item());
                } else {
                    out.files.insert(path, f.item());
                }
            }
        }
        Ok(out)
    }

    fn download(&self, item: &Item) -> Result<Vec<u8>> {
        let url = format!("{}/drive/v3/files/{}?alt=media&supportsAllDrives=true", api(), enc(&item.id));
        let r = reply(P, AGENT.get(&url).header("Authorization", &bearer(&self.token)).call())?;
        if r.ok() { Ok(r.body) } else { Err(super::http::refused(P, r.status, &r.text())) }
    }

    fn create(&self, parent: Option<&str>, name: &str, data: &[u8]) -> Result<Option<Item>> {
        let parent = parent.unwrap_or(&self.root);
        if !self.named(parent, name)?.is_empty() {
            return Ok(None);
        }
        let (ctype, body) = multipart(&serde_json::json!({ "name": name, "parents": [parent] }), data);
        let url = format!("{}/upload/drive/v3/files?uploadType=multipart&supportsAllDrives=true&fields=id,name,mimeType,version", api());
        let f: File = reply(P, AGENT.post(&url).header("Authorization", &bearer(&self.token)).header("Content-Type", &ctype).send(&body[..]))?.expect(P)?;
        Ok(Some(f.item()))
    }

    fn replace(&self, item: &Item, data: &[u8]) -> Result<Option<Item>> {
        // Drive has no conditional upload: check the version first (a race stays possible).
        match file(&self.token, &item.id) {
            Ok(f) if f.version.as_deref() == Some(item.version.as_str()) => {}
            Ok(_) | Err(Error::DriveNoAccess { .. }) => return Ok(None),
            Err(e) => return Err(e),
        }
        let url = format!("{}/upload/drive/v3/files/{}?uploadType=media&supportsAllDrives=true&fields=id,name,mimeType,version", api(), enc(&item.id));
        let f: File = reply(P, AGENT.patch(&url).header("Authorization", &bearer(&self.token)).header("Content-Type", "application/octet-stream").send(data))?.expect(P)?;
        Ok(Some(f.item()))
    }

    fn delete(&self, item: &Item) -> Result<()> {
        match file(&self.token, &item.id) {
            Ok(f) if f.version.as_deref() == Some(item.version.as_str()) => {}
            Ok(_) | Err(Error::DriveNoAccess { .. }) => return Ok(()),
            Err(e) => return Err(e),
        }
        // To the trash, not gone: the owner can still bring it back.
        let url = format!("{}/drive/v3/files/{}?supportsAllDrives=true", api(), enc(&item.id));
        let r = reply(P, AGENT.patch(&url).header("Authorization", &bearer(&self.token)).json_body(&serde_json::json!({ "trashed": true })))?;
        match r.status {
            200..=299 | 403 | 404 => Ok(()),
            s => Err(super::http::refused(P, s, &r.text())),
        }
    }

    fn folder(&self, parent: Option<&str>, name: &str) -> Result<String> {
        let parent = parent.unwrap_or(&self.root);
        if let Some(f) = self.named(parent, name)?.into_iter().find(File::is_folder) {
            return Ok(f.id);
        }
        Ok(make_folder(&self.token, parent, name)?.id)
    }
}

fn make_folder(token: &str, parent: &str, name: &str) -> Result<File> {
    let url = format!("{}/drive/v3/files?supportsAllDrives=true&fields=id,name,mimeType,version", api());
    reply(P, AGENT.post(&url).header("Authorization", &bearer(token)).json_body(&serde_json::json!({ "name": name, "mimeType": FOLDER, "parents": [parent] })))?.expect(P)
}

fn folders_of(files: Vec<File>, shared: bool) -> Vec<DriveFolder> {
    let mut out: Vec<DriveFolder> = files.into_iter().filter(File::is_folder).map(|f| DriveFolder { id: f.id, drive: None, name: f.name, shared }).collect();
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

/// The folders in a folder; at the top, My Drive's and the ones shared with this person.
pub fn browse(token: &str, folder: Option<&DriveRemote>) -> Result<DriveListing> {
    match folder {
        Some(f) => Ok(DriveListing { folder: f.clone(), folders: folders_of(children(token, &f.id)?, false) }),
        None => {
            let mut folders = folders_of(query(token, &format!("'root' in parents and mimeType={} and trashed=false", quoted(FOLDER)), "")?, false);
            folders.extend(folders_of(query(token, &format!("sharedWithMe and mimeType={} and trashed=false", quoted(FOLDER)), "")?, true));
            Ok(DriveListing { folder: DriveRemote { provider: P, id: "root".into(), drive: None, label: String::new(), link: None }, folders })
        }
    }
}

pub fn create_folder(token: &str, parent: Option<&DriveRemote>, name: &str) -> Result<DriveRemote> {
    let f = make_folder(token, parent.map_or("root", |p| p.id.as_str()), name)?;
    let label = match parent {
        Some(p) if !p.label.is_empty() => format!("{}/{}", p.label, f.name),
        _ => f.name.clone(),
    };
    Ok(DriveRemote { provider: P, id: f.id, drive: None, label, link: None })
}

/// The folder id in a Drive link (`/drive/folders/<id>`, `?id=<id>`).
pub fn folder_id(url: &str) -> Option<String> {
    let url = url.trim();
    let id = if let Some(rest) = url.split("/folders/").nth(1) {
        rest.split(['?', '/', '#']).next()?.to_string()
    } else {
        let query = url.split_once('?')?.1;
        query.split('&').find_map(|kv| kv.strip_prefix("id="))?.to_string()
    };
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(id)
}

pub fn open_link(token: &str, url: &str) -> Result<DriveRemote> {
    let id = folder_id(url).ok_or_else(|| Error::Drive { provider: P.id().into(), message: "not a link to a Google Drive folder".into() })?;
    let f = file(token, &id)?;
    if !f.is_folder() {
        return Err(Error::Drive { provider: P.id().into(), message: "the link is not to a folder".into() });
    }
    Ok(DriveRemote { provider: P, id: f.id, drive: None, label: f.name, link: Some(url.trim().to_string()) })
}

pub fn account(token: &str) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct User {
        #[serde(default)]
        email_address: Option<String>,
        #[serde(default)]
        display_name: Option<String>,
    }
    #[derive(Deserialize)]
    struct About {
        user: User,
    }
    let url = format!("{}/drive/v3/about?fields=user(emailAddress,displayName)", api());
    let a: About = reply(P, AGENT.get(&url).header("Authorization", &bearer(token)).call())?.expect(P)?;
    Ok(a.user.email_address.or(a.user.display_name).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_ids_come_from_links() {
        assert_eq!(folder_id("https://drive.google.com/drive/folders/1AbC-d_E?usp=sharing").as_deref(), Some("1AbC-d_E"));
        assert_eq!(folder_id("https://drive.google.com/drive/u/0/folders/1xyz/").as_deref(), Some("1xyz"));
        assert_eq!(folder_id("https://drive.google.com/open?id=1qq&authuser=0").as_deref(), Some("1qq"));
        assert_eq!(folder_id("https://example.com/"), None);
    }

    #[test]
    fn queries_quote_names() {
        assert_eq!(quoted("it's"), r"'it\'s'");
    }
}
