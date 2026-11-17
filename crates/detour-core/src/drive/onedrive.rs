//! OneDrive (personal and work or school) through Microsoft Graph.

use base64::Engine;
use serde::Deserialize;

use super::http::{AGENT, SendJson, bearer, enc, reply};
use super::mirror::{Item, Listing, Storage};
use super::{DriveFolder, DriveListing, DriveRemote, Provider};
use crate::{Error, Result};

const P: Provider = Provider::OneDrive;

fn api() -> String {
    std::env::var("DETOUR_ONEDRIVE_API").unwrap_or_else(|_| "https://graph.microsoft.com/v1.0".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveItem {
    id: String,
    name: String,
    #[serde(default)]
    e_tag: Option<String>,
    #[serde(default)]
    folder: Option<serde_json::Value>,
    #[serde(default)]
    parent_reference: Option<ParentRef>,
    /// A shared item from someone else's drive (in `sharedWithMe`).
    #[serde(default)]
    remote_item: Option<Box<DriveItem>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentRef {
    #[serde(default)]
    drive_id: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    value: Vec<DriveItem>,
    #[serde(rename = "@odata.nextLink", default)]
    next: Option<String>,
}

impl DriveItem {
    fn drive_id(&self) -> Option<String> {
        self.parent_reference.as_ref().and_then(|p| p.drive_id.clone())
    }

    fn item(&self) -> Item {
        Item { id: self.id.clone(), version: self.e_tag.clone().unwrap_or_default() }
    }
}

fn get(url: &str, token: &str) -> Result<super::http::Reply> {
    reply(P, AGENT.get(url).header("Authorization", &bearer(token)).call())
}

/// Every page of a listing.
fn pages(first: String, token: &str) -> Result<Vec<DriveItem>> {
    let mut out = Vec::new();
    let mut url = Some(first);
    while let Some(u) = url {
        let page: Page = get(&u, token)?.expect(P)?;
        out.extend(page.value);
        url = page.next;
    }
    Ok(out)
}

/// The vault's folder, possibly in someone else's drive.
pub struct Folder {
    token: String,
    drive: String,
    root: String,
}

impl Folder {
    pub fn new(token: &str, remote: &DriveRemote) -> Result<Folder> {
        let drive = remote.drive.clone().ok_or_else(|| Error::Drive { provider: P.id().into(), message: "the folder's drive is unknown".into() })?;
        Ok(Folder { token: token.to_string(), drive, root: remote.id.clone() })
    }

    fn item_url(&self, id: &str) -> String {
        format!("{}/drives/{}/items/{}", api(), enc(&self.drive), enc(id))
    }

    fn children(&self, id: &str) -> Result<Vec<DriveItem>> {
        pages(format!("{}/children?$select=id,name,eTag,folder,file&$top=999", self.item_url(id)), &self.token)
    }
}

impl Storage for Folder {
    fn list(&self) -> Result<Listing> {
        let mut out = Listing::default();
        let mut todo = vec![(String::new(), self.root.clone())];
        while let Some((dir, id)) = todo.pop() {
            for c in self.children(&id)? {
                let path = if dir.is_empty() { c.name.clone() } else { format!("{dir}/{}", c.name) };
                if c.folder.is_some() {
                    out.folders.insert(path.clone(), c.id.clone());
                    todo.push((path, c.id));
                } else {
                    out.files.insert(path, c.item());
                }
            }
        }
        Ok(out)
    }

    fn download(&self, item: &Item) -> Result<Vec<u8>> {
        // Graph answers with a redirect to a pre-signed URL, which ureq follows.
        let r = get(&format!("{}/content", self.item_url(&item.id)), &self.token)?;
        if r.ok() { Ok(r.body) } else { Err(super::http::refused(P, r.status, &r.text())) }
    }

    fn create(&self, parent: Option<&str>, name: &str, data: &[u8]) -> Result<Option<Item>> {
        let parent = parent.unwrap_or(&self.root);
        let url = format!("{}:/{}:/content?@microsoft.graph.conflictBehavior=fail", self.item_url(parent), enc(name));
        let r = reply(P, AGENT.put(&url).header("Authorization", &bearer(&self.token)).header("Content-Type", "application/octet-stream").send(data))?;
        if r.status == 409 {
            return Ok(None);
        }
        let it: DriveItem = r.expect(P)?;
        Ok(Some(it.item()))
    }

    fn replace(&self, item: &Item, data: &[u8]) -> Result<Option<Item>> {
        let r = reply(
            P,
            AGENT
                .put(&format!("{}/content", self.item_url(&item.id)))
                .header("Authorization", &bearer(&self.token))
                .header("If-Match", &item.version)
                .header("Content-Type", "application/octet-stream")
                .send(data),
        )?;
        if r.status == 412 || r.status == 409 || r.status == 404 {
            return Ok(None);
        }
        let it: DriveItem = r.expect(P)?;
        Ok(Some(it.item()))
    }

    fn delete(&self, item: &Item) -> Result<()> {
        let r = reply(P, AGENT.delete(&self.item_url(&item.id)).header("Authorization", &bearer(&self.token)).header("If-Match", &item.version).call())?;
        match r.status {
            200..=299 | 404 | 409 | 412 => Ok(()),
            s => Err(super::http::refused(P, s, &r.text())),
        }
    }

    fn folder(&self, parent: Option<&str>, name: &str) -> Result<String> {
        let parent = parent.unwrap_or(&self.root);
        let body = serde_json::json!({ "name": name, "folder": {}, "@microsoft.graph.conflictBehavior": "fail" });
        let r = reply(P, AGENT.post(&format!("{}/children", self.item_url(parent))).header("Authorization", &bearer(&self.token)).json_body(&body))?;
        if r.status == 409 {
            let it: DriveItem = get(&format!("{}:/{}", self.item_url(parent), enc(name)), &self.token)?.expect(P)?;
            return Ok(it.id);
        }
        let it: DriveItem = r.expect(P)?;
        Ok(it.id)
    }
}

fn remote_of(it: &DriveItem, link: Option<String>) -> Result<DriveRemote> {
    let drive = it.drive_id().ok_or_else(|| Error::Drive { provider: P.id().into(), message: "the folder's drive is unknown".into() })?;
    Ok(DriveRemote { provider: P, id: it.id.clone(), drive: Some(drive), label: it.name.clone(), link })
}

fn folders_of(items: Vec<DriveItem>) -> Vec<DriveFolder> {
    let mut out: Vec<DriveFolder> = items
        .into_iter()
        .filter_map(|it| {
            if it.folder.is_some() {
                return Some(DriveFolder { drive: it.drive_id(), id: it.id, name: it.name, shared: false });
            }
            // Shared with this person: the folder lives in another drive.
            let r = it.remote_item?;
            r.folder.as_ref()?;
            Some(DriveFolder { drive: r.drive_id(), id: r.id, name: it.name, shared: true })
        })
        .collect();
    out.sort_by(|a, b| (a.shared, a.name.to_lowercase()).cmp(&(b.shared, b.name.to_lowercase())));
    out
}

/// The folders in a folder; at the top, this person's drive and what is shared with them.
pub fn browse(token: &str, folder: Option<&DriveRemote>) -> Result<DriveListing> {
    match folder {
        Some(f) => {
            let drive = f.drive.clone().unwrap_or_default();
            let items = pages(format!("{}/drives/{}/items/{}/children?$select=id,name,folder,parentReference&$top=999", api(), enc(&drive), enc(&f.id)), token)?;
            Ok(DriveListing { folder: f.clone(), folders: folders_of(items).into_iter().map(|mut x| { x.drive = x.drive.or_else(|| Some(drive.clone())); x }).collect() })
        }
        None => {
            let root: DriveItem = get(&format!("{}/me/drive/root?$select=id,name,parentReference", api()), token)?.expect(P)?;
            let mut folders = folders_of(pages(format!("{}/me/drive/root/children?$select=id,name,folder,parentReference&$top=999", api()), token)?);
            // Shared folders; not every account offers this list, which is fine.
            if let Ok(shared) = pages(format!("{}/me/drive/sharedWithMe", api()), token) {
                folders.extend(folders_of(shared).into_iter().filter(|f| f.shared));
            }
            let drive = root.drive_id();
            Ok(DriveListing { folder: DriveRemote { provider: P, id: root.id, drive, label: String::new(), link: None }, folders })
        }
    }
}

pub fn create_folder(token: &str, parent: Option<&DriveRemote>, name: &str) -> Result<DriveRemote> {
    let url = match parent {
        Some(p) => format!("{}/drives/{}/items/{}/children", api(), enc(p.drive.as_deref().unwrap_or_default()), enc(&p.id)),
        None => format!("{}/me/drive/root/children", api()),
    };
    let body = serde_json::json!({ "name": name, "folder": {}, "@microsoft.graph.conflictBehavior": "fail" });
    let r = reply(P, AGENT.post(&url).header("Authorization", &bearer(token)).json_body(&body))?;
    if r.status == 409 {
        return Err(Error::Drive { provider: P.id().into(), message: format!("{name} already exists") });
    }
    let it: DriveItem = r.expect(P)?;
    let mut remote = remote_of(&it, None)?;
    if let Some(p) = parent
        && !p.label.is_empty()
    {
        remote.label = format!("{}/{}", p.label, it.name);
    }
    Ok(remote)
}

/// Graph's id for a share link: `u!` and the URL in unpadded base64url.
pub fn share_id(url: &str) -> String {
    format!("u!{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url.trim().as_bytes()))
}

/// The folder a share link points to; the link is redeemed, so the folder stays reachable.
pub fn open_link(token: &str, url: &str) -> Result<DriveRemote> {
    let r = reply(
        P,
        AGENT
            .get(&format!("{}/shares/{}/driveItem?$select=id,name,folder,parentReference", api(), share_id(url)))
            .header("Authorization", &bearer(token))
            .header("Prefer", "redeemSharingLink")
            .call(),
    )?;
    let it: DriveItem = r.expect(P)?;
    if it.folder.is_none() {
        return Err(Error::Drive { provider: P.id().into(), message: "the link is not to a folder".into() });
    }
    remote_of(&it, Some(url.trim().to_string()))
}

/// The signed-in account's address or name.
pub fn account(token: &str) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Me {
        #[serde(default)]
        mail: Option<String>,
        #[serde(default)]
        user_principal_name: Option<String>,
        #[serde(default)]
        display_name: Option<String>,
    }
    let me: Me = get(&format!("{}/me?$select=mail,userPrincipalName,displayName", api()), token)?.expect(P)?;
    Ok(me.mail.or(me.user_principal_name).or(me.display_name).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_ids_are_unpadded_base64url() {
        // The example in Graph's documentation.
        assert_eq!(share_id("https://onedrive.live.com/redir?resid=1231244193912!12&authKey=1201919!12921!1"), "u!aHR0cHM6Ly9vbmVkcml2ZS5saXZlLmNvbS9yZWRpcj9yZXNpZD0xMjMxMjQ0MTkzOTEyITEyJmF1dGhLZXk9MTIwMTkxOSExMjkyMSEx");
    }

    #[test]
    fn listings_parse_folders_files_and_shared_items() {
        let page: Page = serde_json::from_str(r#"{"value":[
            {"id":"A","name":"ops","folder":{"childCount":1},"parentReference":{"driveId":"d1"}},
            {"id":"B","name":"vault.json","eTag":"\"{B},1\"","file":{}},
            {"id":"S","name":"Team","remoteItem":{"id":"R","name":"Team","folder":{},"parentReference":{"driveId":"d2"}}}
        ],"@odata.nextLink":"https://next"}"#)
        .unwrap();
        assert_eq!(page.next.as_deref(), Some("https://next"));
        assert_eq!(page.value[1].item(), Item { id: "B".into(), version: "\"{B},1\"".into() });
        let folders = folders_of(page.value);
        assert_eq!(folders.iter().map(|f| (f.id.as_str(), f.drive.as_deref(), f.shared)).collect::<Vec<_>>(), vec![("A", Some("d1"), false), ("R", Some("d2"), true)]);
    }
}
