//! Dropbox through API v2. A folder is addressed by its path in the signed-in account's
//! Dropbox; a folder shared with this person is used once it is in their Dropbox.

use serde::Deserialize;

use super::http::{AGENT, SendJson, bearer, reply};
use super::mirror::{Item, Listing, Storage};
use super::{DriveFolder, DriveListing, DriveRemote, Provider};
use crate::{Error, Result};

const P: Provider = Provider::Dropbox;

fn api() -> String {
    std::env::var("DETOUR_DROPBOX_API").unwrap_or_else(|_| "https://api.dropboxapi.com".into())
}

fn content() -> String {
    std::env::var("DETOUR_DROPBOX_CONTENT").unwrap_or_else(|_| "https://content.dropboxapi.com".into())
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = ".tag")]
    tag: String,
    name: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    path_display: Option<String>,
    #[serde(default)]
    path_lower: Option<String>,
    #[serde(default)]
    rev: Option<String>,
    #[serde(default)]
    shared_folder_id: Option<String>,
    #[serde(default)]
    sharing_info: Option<SharingInfo>,
}

#[derive(Deserialize)]
struct SharingInfo {
    #[serde(default)]
    shared_folder_id: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    entries: Vec<Entry>,
    cursor: String,
    has_more: bool,
}

/// An RPC endpoint: JSON in, JSON out.
fn rpc(token: &str, route: &str, args: &serde_json::Value) -> Result<super::http::Reply> {
    reply(P, AGENT.post(&format!("{}/2/{route}", api())).header("Authorization", &bearer(token)).json_body(args))
}

/// Is a 409's error the given kind (`path/conflict`, `path/not_found`, ...)?
fn error_is(r: &super::http::Reply, what: &str) -> bool {
    r.status == 409 && r.text().contains(what)
}

/// The arguments header of a content endpoint: JSON, with non-ASCII escaped as Dropbox asks.
fn arg_header(v: &serde_json::Value) -> String {
    let mut out = String::new();
    for c in v.to_string().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

fn list(token: &str, path: &str, recursive: bool) -> Result<Vec<Entry>> {
    let mut page: Page = rpc(token, "files/list_folder", &serde_json::json!({ "path": path, "recursive": recursive, "include_deleted": false }))?.expect(P)?;
    let mut out = std::mem::take(&mut page.entries);
    while page.has_more {
        page = rpc(token, "files/list_folder/continue", &serde_json::json!({ "cursor": page.cursor }))?.expect(P)?;
        out.append(&mut page.entries);
    }
    Ok(out)
}

/// The vault's folder: its path in this account's Dropbox.
pub struct Folder {
    token: String,
    root: String,
}

impl Folder {
    pub fn new(token: &str, remote: &DriveRemote) -> Result<Folder> {
        Ok(Folder { token: token.to_string(), root: remote.id.trim_end_matches('/').to_string() })
    }

    /// Folder "ids" here are paths; `None` is the vault's folder.
    fn path_in(&self, parent: Option<&str>, name: &str) -> String {
        format!("{}/{name}", parent.unwrap_or(&self.root).trim_end_matches('/'))
    }

    fn upload(&self, path: &str, mode: serde_json::Value, data: &[u8]) -> Result<super::http::Reply> {
        let args = serde_json::json!({ "path": path, "mode": mode, "autorename": false, "strict_conflict": true, "mute": true });
        reply(
            P,
            AGENT
                .post(&format!("{}/2/files/upload", content()))
                .header("Authorization", &bearer(&self.token))
                .header("Dropbox-API-Arg", &arg_header(&args))
                .header("Content-Type", "application/octet-stream")
                .send(data),
        )
    }
}

fn item_of(e: &Entry) -> Item {
    Item { id: e.id.clone(), version: e.rev.clone().unwrap_or_default() }
}

impl Storage for Folder {
    fn list(&self) -> Result<Listing> {
        let mut out = Listing::default();
        let root = self.root.to_lowercase();
        for e in list(&self.token, &self.root, true)? {
            let lower = e.path_lower.clone().unwrap_or_default();
            let Some(rel) = lower.strip_prefix(&root).map(|r| r.trim_start_matches('/')) else { continue };
            if rel.is_empty() {
                continue;
            }
            // Keep the case the files were written with.
            let display = e.path_display.clone().unwrap_or_else(|| lower.clone());
            let rel = &display[display.len() - rel.len()..];
            match e.tag.as_str() {
                "folder" => {
                    out.folders.insert(rel.to_string(), display.clone());
                }
                "file" => {
                    out.files.insert(rel.to_string(), item_of(&e));
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn download(&self, item: &Item) -> Result<Vec<u8>> {
        let r = reply(
            P,
            AGENT
                .post(&format!("{}/2/files/download", content()))
                .header("Authorization", &bearer(&self.token))
                .header("Dropbox-API-Arg", &arg_header(&serde_json::json!({ "path": item.id })))
                .send_empty(),
        )?;
        if r.ok() { Ok(r.body) } else { Err(super::http::refused(P, r.status, &r.text())) }
    }

    fn create(&self, parent: Option<&str>, name: &str, data: &[u8]) -> Result<Option<Item>> {
        let r = self.upload(&self.path_in(parent, name), serde_json::json!("add"), data)?;
        if error_is(&r, "conflict") {
            return Ok(None);
        }
        let e: Entry = with_tag(r.expect::<serde_json::Value>(P)?, "file")?;
        Ok(Some(item_of(&e)))
    }

    fn replace(&self, item: &Item, data: &[u8]) -> Result<Option<Item>> {
        let meta = rpc(&self.token, "files/get_metadata", &serde_json::json!({ "path": item.id }))?;
        if !meta.ok() {
            return Ok(None);
        }
        let current: Entry = meta.json(P)?;
        let path = current.path_display.clone().unwrap_or_default();
        let r = self.upload(&path, serde_json::json!({ ".tag": "update", "update": item.version }), data)?;
        if error_is(&r, "conflict") {
            return Ok(None);
        }
        let e: Entry = with_tag(r.expect::<serde_json::Value>(P)?, "file")?;
        Ok(Some(item_of(&e)))
    }

    fn delete(&self, item: &Item) -> Result<()> {
        let r = rpc(&self.token, "files/delete_v2", &serde_json::json!({ "path": item.id, "parent_rev": item.version }))?;
        if r.ok() || r.status == 409 { Ok(()) } else { Err(super::http::refused(P, r.status, &r.text())) }
    }

    fn folder(&self, parent: Option<&str>, name: &str) -> Result<String> {
        let path = self.path_in(parent, name);
        let r = rpc(&self.token, "files/create_folder_v2", &serde_json::json!({ "path": path, "autorename": false }))?;
        if r.ok() || error_is(&r, "conflict") {
            return Ok(path);
        }
        Err(super::http::refused(P, r.status, &r.text()))
    }
}

/// Upload answers are file metadata without the `.tag` the listings carry.
fn with_tag(mut v: serde_json::Value, tag: &str) -> Result<Entry> {
    if let Some(o) = v.as_object_mut() {
        o.entry(".tag").or_insert_with(|| serde_json::json!(tag));
    }
    serde_json::from_value(v).map_err(|e| Error::Drive { provider: P.id().into(), message: format!("unexpected answer: {e}") })
}

fn remote_of(e: &Entry, link: Option<String>) -> DriveRemote {
    let shared = e.shared_folder_id.clone().or_else(|| e.sharing_info.as_ref().and_then(|s| s.shared_folder_id.clone()));
    let path = e.path_display.clone().or_else(|| e.path_lower.clone()).unwrap_or_default();
    DriveRemote { provider: P, label: path.trim_start_matches('/').to_string(), id: path, drive: shared, link }
}

pub fn browse(token: &str, folder: Option<&DriveRemote>) -> Result<DriveListing> {
    let path = folder.map(|f| f.id.clone()).unwrap_or_default();
    let mut folders: Vec<DriveFolder> = list(token, &path, false)?
        .into_iter()
        .filter(|e| e.tag == "folder")
        .map(|e| {
            let shared = e.shared_folder_id.is_some() || e.sharing_info.is_some();
            DriveFolder { id: e.path_display.clone().unwrap_or_default(), drive: None, name: e.name, shared }
        })
        .collect();
    folders.sort_by_key(|f| f.name.to_lowercase());
    let here = folder.cloned().unwrap_or(DriveRemote { provider: P, id: String::new(), drive: None, label: String::new(), link: None });
    Ok(DriveListing { folder: here, folders })
}

pub fn create_folder(token: &str, parent: Option<&DriveRemote>, name: &str) -> Result<DriveRemote> {
    let path = format!("{}/{name}", parent.map(|p| p.id.trim_end_matches('/')).unwrap_or(""));
    let r = rpc(token, "files/create_folder_v2", &serde_json::json!({ "path": path, "autorename": false }))?;
    if error_is(&r, "conflict") {
        return Err(Error::Drive { provider: P.id().into(), message: format!("{name} already exists") });
    }
    #[derive(Deserialize)]
    struct Created {
        metadata: serde_json::Value,
    }
    let c: Created = r.expect(P)?;
    Ok(remote_of(&with_tag(c.metadata, "folder")?, None))
}

/// The folder a shared link points to, once it is in this person's Dropbox (Dropbox doesn't
/// let apps write through a link to a folder).
pub fn open_link(token: &str, url: &str) -> Result<DriveRemote> {
    let r = rpc(token, "sharing/get_shared_link_metadata", &serde_json::json!({ "url": url.trim() }))?;
    let e: Entry = r.expect(P)?;
    if e.tag != "folder" {
        return Err(Error::Drive { provider: P.id().into(), message: "the link is not to a folder".into() });
    }
    if e.path_lower.is_none() {
        return Err(Error::Drive { provider: P.id().into(), message: format!("add \"{}\" to your Dropbox first", e.name) });
    }
    Ok(remote_of(&e, Some(url.trim().to_string())))
}

pub fn account(token: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Account {
        #[serde(default)]
        email: Option<String>,
    }
    let r = reply(P, AGENT.post(&format!("{}/2/users/get_current_account", api())).header("Authorization", &bearer(token)).header("Content-Type", "application/json").send("null"))?;
    let a: Account = r.expect(P)?;
    Ok(a.email.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_headers_escape_non_ascii() {
        assert_eq!(arg_header(&serde_json::json!({ "path": "/\u{41a}\u{43e}" })), r#"{"path":"/\u041a\u043e"}"#);
    }

    #[test]
    fn listings_parse() {
        let p: Page = serde_json::from_str(r#"{"entries":[
            {".tag":"folder","name":"ops","id":"id:1","path_lower":"/team/ops","path_display":"/Team/ops"},
            {".tag":"file","name":"vault.json","id":"id:2","path_lower":"/team/vault.json","path_display":"/Team/vault.json","rev":"015f"}
        ],"cursor":"c","has_more":false}"#)
        .unwrap();
        assert_eq!(item_of(&p.entries[1]), Item { id: "id:2".into(), version: "015f".into() });
    }
}
