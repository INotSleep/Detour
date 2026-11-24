//! Vaults kept in a cloud drive (OneDrive, Google Drive, Dropbox), reached through the
//! drive's own API: no sync client on the device, no server of Detour's.
//!
//! As with Git (`team::git`), a vault is kept as a working copy (`<dir>/files`, the layout
//! of a folder vault) that `Mirror` pulls into and pushes from. The drive sees only what a
//! shared folder would: encrypted, signed entries.

pub mod auth;
pub mod dropbox;
pub mod gdrive;
pub mod link;
pub mod mirror;
pub mod onedrive;
#[doc(hidden)]
pub mod fake;

use serde::{Deserialize, Serialize};

pub use mirror::{Mirror, Storage};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[serde(rename = "onedrive")]
    OneDrive,
    #[serde(rename = "gdrive")]
    GoogleDrive,
    #[serde(rename = "dropbox")]
    Dropbox,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::OneDrive => "OneDrive",
            Provider::GoogleDrive => "Google Drive",
            Provider::Dropbox => "Dropbox",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Provider::OneDrive => "onedrive",
            Provider::GoogleDrive => "gdrive",
            Provider::Dropbox => "dropbox",
        }
    }

    pub fn parse(s: &str) -> Option<Provider> {
        match s.trim().to_ascii_lowercase().as_str() {
            "onedrive" => Some(Provider::OneDrive),
            "gdrive" | "google" | "googledrive" | "google-drive" => Some(Provider::GoogleDrive),
            "dropbox" => Some(Provider::Dropbox),
            _ => None,
        }
    }

    /// The API host, for `Unreachable` errors.
    pub fn endpoint(self) -> &'static str {
        match self {
            Provider::OneDrive => "graph.microsoft.com",
            Provider::GoogleDrive => "www.googleapis.com",
            Provider::Dropbox => "api.dropboxapi.com",
        }
    }
}

/// Where a vault's folder is in a drive. What the ids mean is the provider's business:
/// OneDrive: `drive` + `id` (an item, possibly in someone else's drive); Google Drive: `id`
/// (a folder's file id); Dropbox: `id` is the folder's path (`/Detour/team`), `drive` a
/// shared folder's id when it is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveRemote {
    pub provider: Provider,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<String>,
    /// What the user sees: the folder's path or name.
    #[serde(default)]
    pub label: String,
    /// The share link it was opened with, if any (OneDrive redeems it again when needed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The account it is reached with, of those this device is signed in to (any of that
    /// drive's when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl DriveRemote {
    /// "OneDrive: Detour/Team", for lists and status lines.
    pub fn display(&self) -> String {
        if self.label.is_empty() { self.provider.name().to_string() } else { format!("{}: {}", self.provider.name(), self.label) }
    }
}

/// This device's sign-in to a drive; never syncs (each device signs in itself).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveLogin {
    pub provider: Provider,
    /// The account's e-mail or name, to show.
    pub account: String,
    /// Empty when the drive refused to renew it: signing in again is needed.
    pub access_token: String,
    /// Unix ms.
    pub expires_at: i64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for DriveLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveLogin").field("provider", &self.provider).field("account", &self.account).field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// A folder in a drive, for browsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFolder {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<String>,
    pub name: String,
    /// Shared with this person by someone else.
    #[serde(default)]
    pub shared: bool,
}

/// What browsing a folder shows: where it is and the folders in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveListing {
    pub folder: DriveRemote,
    pub folders: Vec<DriveFolder>,
}

/// Folders served from memory instead of a real drive, by folder id (tests only).
#[doc(hidden)]
pub static FAKES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, fake::FakeDrive>>> = std::sync::LazyLock::new(Default::default);

/// Talks to one provider's API with a signed-in token.
pub fn storage(remote: &DriveRemote, token: &str) -> Result<Box<dyn Storage>> {
    if let Some(f) = FAKES.lock().unwrap_or_else(|e| e.into_inner()).get(&remote.id) {
        return Ok(Box::new(f.folder()));
    }
    Ok(match remote.provider {
        Provider::OneDrive => Box::new(onedrive::Folder::new(token, remote)?),
        Provider::GoogleDrive => Box::new(gdrive::Folder::new(token, remote)?),
        Provider::Dropbox => Box::new(dropbox::Folder::new(token, remote)?),
    })
}

/// Lists the folders in `folder` (`None`: the drive's top, with folders shared with this person).
pub fn browse(provider: Provider, token: &str, folder: Option<&DriveRemote>) -> Result<DriveListing> {
    match provider {
        Provider::OneDrive => onedrive::browse(token, folder),
        Provider::GoogleDrive => gdrive::browse(token, folder),
        Provider::Dropbox => dropbox::browse(token, folder),
    }
}

/// Makes a folder in `parent`, returning it.
pub fn create_folder(provider: Provider, token: &str, parent: Option<&DriveRemote>, name: &str) -> Result<DriveRemote> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(Error::Drive { provider: provider.id().into(), message: "invalid folder name".into() });
    }
    match provider {
        Provider::OneDrive => onedrive::create_folder(token, parent, name),
        Provider::GoogleDrive => gdrive::create_folder(token, parent, name),
        Provider::Dropbox => dropbox::create_folder(token, parent, name),
    }
}

/// The folder a share link points to.
pub fn open_link(provider: Provider, token: &str, url: &str) -> Result<DriveRemote> {
    match provider {
        Provider::OneDrive => onedrive::open_link(token, url),
        Provider::GoogleDrive => gdrive::open_link(token, url),
        Provider::Dropbox => dropbox::open_link(token, url),
    }
}

/* ---------- HTTP, shared by the providers ---------- */

pub(crate) mod http {
    use std::sync::LazyLock;
    use std::time::Duration;

    use super::Provider;
    use crate::Error;

    pub static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
            .proxy(ureq::Proxy::try_from_env())
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .user_agent(concat!("detour/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    });

    /// A network failure as the UI shows it: unreachable, or another connection problem.
    pub fn net_error(p: Provider, e: ureq::Error) -> Error {
        // What failed, for the log: "no connection" alone tells nothing of why.
        crate::logfile::write("warn", &format!("{}: {e}", p.endpoint()));
        match e {
            ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => Error::Unreachable { endpoint: p.endpoint().into() },
            other => Error::Connect { message: other.to_string() },
        }
    }

    /// An answer: its status and body.
    pub struct Reply {
        pub status: u16,
        pub body: Vec<u8>,
    }

    impl Reply {
        pub fn ok(&self) -> bool {
            (200..300).contains(&self.status)
        }

        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.body).to_string()
        }

        pub fn json<T: serde::de::DeserializeOwned>(&self, p: Provider) -> Result<T, Error> {
            serde_json::from_slice(&self.body).map_err(|e| Error::Drive { provider: p.id().into(), message: format!("unexpected answer: {e}") })
        }

        /// The body as JSON if the request went through, otherwise the refusal.
        pub fn expect<T: serde::de::DeserializeOwned>(self, p: Provider) -> Result<T, Error> {
            if self.ok() { self.json(p) } else { Err(refused(p, self.status, &self.text())) }
        }
    }

    /// Reads a response whole (vault files are small).
    pub fn reply(p: Provider, r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Reply, Error> {
        let mut resp = r.map_err(|e| net_error(p, e))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| net_error(p, e))?;
        Ok(Reply { status, body })
    }

    /// JSON bodies without ureq's `json` feature.
    pub trait SendJson {
        fn json_body(self, v: &serde_json::Value) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>;
    }

    impl SendJson for ureq::RequestBuilder<ureq::typestate::WithBody> {
        fn json_body(self, v: &serde_json::Value) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
            self.header("Content-Type", "application/json").send(v.to_string())
        }
    }

    pub fn bearer(token: &str) -> String {
        format!("Bearer {token}")
    }

    /// `%`-encodes a URL query value or path segment.
    pub fn enc(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    /// An answer the drive refused: 401 means signing in again, 403/404 no access.
    pub fn refused(p: Provider, status: u16, body: &str) -> Error {
        match status {
            401 => Error::DriveSignIn { provider: p.id().into(), account: None },
            403 | 404 => Error::DriveNoAccess { provider: p.id().into() },
            _ => {
                let short: String = body.chars().take(300).collect();
                Error::Drive { provider: p.id().into(), message: format!("{status} {short}") }
            }
        }
    }
}
