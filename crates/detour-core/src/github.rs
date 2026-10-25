//! Signing in with GitHub through the Detour Sync GitHub App, with the device
//! flow: no client secret and no Detour server.
//!
//! The token reaches only the repositories the app is installed on, with read
//! and write access to their contents, and only those the person can reach
//! anyway. It lives in the personal vault of this device (it never syncs to
//! other devices: renewing it replaces the refresh token, which two devices
//! can't share), expires after hours and is renewed before Git operations.

use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::model::{Millis, now_ms};
use crate::store::Store;
use crate::{Error, Result};

/// The Detour Sync GitHub App (public: device flow needs no secret).
pub const CLIENT_ID: &str = "Iv23li4wtqpgIkO66jtR";
/// A token this close to expiring is renewed before it is used.
const RENEW_BEFORE_MS: Millis = 10 * 60 * 1000;

/// This device's GitHub sign-in.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubLogin {
    pub user: String,
    /// Empty once GitHub refused to renew it: signing in again is needed.
    pub access_token: String,
    /// 0: never expires.
    #[serde(default)]
    pub expires_at: Millis,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub refresh_expires_at: Millis,
}

impl std::fmt::Debug for GithubLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubLogin").field("user", &self.user).field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

impl GithubLogin {
    fn needs_renewal(&self, now: Millis) -> bool {
        self.expires_at != 0 && now + RENEW_BEFORE_MS >= self.expires_at
    }

    fn can_renew(&self, now: Millis) -> bool {
        self.refresh_token.as_deref().is_some_and(|t| !t.is_empty()) && (self.refresh_expires_at == 0 || now < self.refresh_expires_at)
    }
}

/// The code a person enters on GitHub to let this device in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds the code is valid.
    pub expires_in: u64,
    /// Seconds to wait between polls.
    pub interval: u64,
}

/// Where a sign-in stands after a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Poll {
    Pending,
    /// GitHub asks to poll less often, every `interval` seconds.
    SlowDown { interval: u64 },
    Done { user: String },
}

/// GitHub's web and API hosts; tests point them at a local server.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub web: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints { web: "https://github.com".into(), api: "https://api.github.com".into() }
    }
}

#[derive(Deserialize)]
struct DeviceReply {
    device_code: Option<String>,
    user_code: Option<String>,
    verification_uri: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<i64>,
    error: Option<String>,
    interval: Option<u64>,
}

impl TokenReply {
    fn login(self, user: String, now: Millis) -> Option<GithubLogin> {
        let at = |secs: Option<i64>| secs.filter(|s| *s > 0).map_or(0, |s| now + s * 1000);
        Some(GithubLogin {
            user,
            access_token: self.access_token.filter(|t| !t.is_empty())?,
            expires_at: at(self.expires_in),
            refresh_token: self.refresh_token.filter(|t| !t.is_empty()),
            refresh_expires_at: at(self.refresh_token_expires_in),
        })
    }
}

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .proxy(ureq::Proxy::try_from_env())
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(concat!("detour/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
});

/// Only one renewal at a time: each one replaces the refresh token.
static RENEWING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn locked(store: &Mutex<Store>) -> Result<MutexGuard<'_, Store>> {
    store.lock().map_err(|_| Error::Internal { message: "state lock poisoned".into() })
}

fn git(message: &str) -> Error {
    Error::Git { message: message.into() }
}

fn net_error(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => Error::Unreachable { endpoint: "github.com".into() },
        other => Error::Connect { message: other.to_string() },
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::Internal { message: e.to_string() })?
}

/// POSTs a form to GitHub's OAuth endpoints, which answer JSON (errors too, with status 200).
async fn post_form<T: serde::de::DeserializeOwned + Send + 'static>(url: String, form: Vec<(&'static str, String)>) -> Result<T> {
    blocking(move || {
        let mut resp = AGENT.post(&url).header("Accept", "application/json").send_form(form.iter().map(|(k, v)| (*k, v.as_str()))).map_err(net_error)?;
        let body = resp.body_mut().read_to_string().map_err(net_error)?;
        serde_json::from_str(&body).map_err(|_| git(&format!("github answered {}", resp.status().as_u16())))
    })
    .await
}

/// GETs from GitHub's API with a token: the status and the body.
async fn api_get(url: String, token: String) -> Result<(u16, String)> {
    blocking(move || {
        let mut resp = AGENT
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(net_error)?;
        let body = resp.body_mut().read_to_string().map_err(net_error)?;
        Ok((resp.status().as_u16(), body))
    })
    .await
}

/// Starts a sign-in: the code to show and how often to poll.
pub async fn start(ep: &Endpoints) -> Result<DeviceCode> {
    let r: DeviceReply = post_form(format!("{}/login/device/code", ep.web), vec![("client_id", CLIENT_ID.into())]).await?;
    if let Some(e) = r.error {
        return Err(git(&format!("github: {e}")));
    }
    match (r.device_code, r.user_code, r.verification_uri) {
        (Some(device_code), Some(user_code), Some(verification_uri)) => Ok(DeviceCode { device_code, user_code, verification_uri, expires_in: r.expires_in.unwrap_or(900), interval: r.interval.unwrap_or(5) }),
        _ => Err(git("github: unexpected answer")),
    }
}

/// Asks whether the person has entered the code yet; once they have, keeps the sign-in.
pub async fn poll(store: &Mutex<Store>, ep: &Endpoints, device_code: &str) -> Result<Poll> {
    let form = vec![("client_id", CLIENT_ID.into()), ("device_code", device_code.to_string()), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code".into())];
    let r: TokenReply = post_form(format!("{}/login/oauth/access_token", ep.web), form).await?;
    match r.error.as_deref() {
        Some("authorization_pending") => return Ok(Poll::Pending),
        Some("slow_down") => return Ok(Poll::SlowDown { interval: r.interval.unwrap_or(10) }),
        Some("expired_token") => return Err(git("github sign-in expired")),
        Some("access_denied") => return Err(git("github sign-in denied")),
        Some(e) => return Err(git(&format!("github: {e}"))),
        None => {}
    }
    let token = r.access_token.clone().filter(|t| !t.is_empty()).ok_or_else(|| git("github: unexpected answer"))?;
    let user = user_of(ep, &token).await?;
    let login = r.login(user.clone(), now_ms()).ok_or_else(|| git("github: unexpected answer"))?;
    locked(store)?.set_github_login(Some(login))?;
    Ok(Poll::Done { user })
}

async fn user_of(ep: &Endpoints, token: &str) -> Result<String> {
    let (status, body) = api_get(format!("{}/user", ep.api), token.to_string()).await?;
    if status != 200 {
        return Err(git("github sign-in needed"));
    }
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    serde_json::from_str::<User>(&body).map(|u| u.login).map_err(|_| git("github: unexpected answer"))
}

/// Renews this device's GitHub token when it is about to expire. A network
/// failure keeps the old one (the Git operation then reports the network);
/// a refused renewal leaves a sign-in without a token, so Git asks to sign in.
pub async fn renew(store: &Mutex<Store>, ep: &Endpoints) -> Result<()> {
    let _one = RENEWING.lock().await;
    let Some(login) = locked(store)?.github_login()? else { return Ok(()) };
    let now = now_ms();
    if login.access_token.is_empty() || !login.needs_renewal(now) {
        return Ok(());
    }
    if !login.can_renew(now) {
        return locked(store)?.set_github_login(Some(GithubLogin { access_token: String::new(), refresh_token: None, ..login }));
    }
    let form = vec![("client_id", CLIENT_ID.into()), ("grant_type", "refresh_token".into()), ("refresh_token", login.refresh_token.clone().unwrap_or_default())];
    let reply: TokenReply = match post_form(format!("{}/login/oauth/access_token", ep.web), form).await {
        Ok(r) => r,
        Err(Error::Unreachable { .. } | Error::Connect { .. }) => return Ok(()),
        Err(e) => return Err(e),
    };
    let renewed = if reply.error.is_none() { reply.login(login.user.clone(), now_ms()) } else { None };
    let next = renewed.unwrap_or(GithubLogin { access_token: String::new(), refresh_token: None, ..login });
    locked(store)?.set_github_login(Some(next))
}

/// `owner/repo` of a github.com repository given by its https address.
pub fn repo_of(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix("https://github.com/")?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (ok(owner) && ok(repo)).then(|| (owner.to_string(), repo.to_string()))
}

/// Whether this device's GitHub sign-in reaches a repository: the app is
/// installed on it and the person has access.
pub async fn repo_access(store: &Mutex<Store>, ep: &Endpoints, url: &str) -> Result<bool> {
    let (owner, repo) = repo_of(url).ok_or_else(|| git("not a github.com repository"))?;
    renew(store, ep).await?;
    let token = locked(store)?.github_login()?.map(|l| l.access_token).filter(|t| !t.is_empty()).ok_or_else(|| git("github sign-in needed"))?;
    let (status, _) = api_get(format!("{}/repos/{owner}/{repo}", ep.api), token).await?;
    match status {
        200 => Ok(true),
        403 | 404 => Ok(false),
        401 => Err(git("github sign-in needed")),
        other => Err(git(&format!("github answered {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(json: &str) -> TokenReply {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_token_reply_becomes_a_login_with_absolute_expiry() {
        let r = reply(r#"{"access_token":"ghu_a","expires_in":28800,"refresh_token":"ghr_b","refresh_token_expires_in":15897600,"token_type":"bearer","scope":""}"#);
        let l = r.login("me".into(), 1_000).unwrap();
        assert_eq!((l.access_token.as_str(), l.expires_at, l.refresh_token.as_deref(), l.refresh_expires_at), ("ghu_a", 1_000 + 28_800_000, Some("ghr_b"), 1_000 + 15_897_600_000));
        // A token that never expires (the app opted out).
        let l = reply(r#"{"access_token":"ghu_a"}"#).login("me".into(), 1_000).unwrap();
        assert_eq!((l.expires_at, l.refresh_token.as_deref()), (0, None));
        assert!(!l.needs_renewal(i64::MAX - RENEW_BEFORE_MS));
        assert!(reply(r#"{"error":"authorization_pending"}"#).login("me".into(), 0).is_none());
    }

    #[test]
    fn renewal_starts_ten_minutes_early_and_needs_a_live_refresh_token() {
        let l = reply(r#"{"access_token":"a","expires_in":3600,"refresh_token":"r","refresh_token_expires_in":7200}"#).login("me".into(), 0).unwrap();
        assert!(!l.needs_renewal(0));
        assert!(l.needs_renewal(3_600_000 - RENEW_BEFORE_MS));
        assert!(l.can_renew(7_199_999));
        assert!(!l.can_renew(7_200_000));
    }

    #[test]
    fn github_repositories_are_recognised_by_their_https_address() {
        assert_eq!(repo_of("https://github.com/acme/team-vault.git"), Some(("acme".into(), "team-vault".into())));
        assert_eq!(repo_of(" https://github.com/acme/vault/ "), Some(("acme".into(), "vault".into())));
        for other in ["git@github.com:acme/vault.git", "https://gitlab.com/acme/vault", "https://github.com/acme", "https://github.com/acme/vault/tree/main", "https://github.com/a b/c"] {
            assert_eq!(repo_of(other), None, "{other}");
        }
    }

    #[test]
    fn a_login_never_shows_its_tokens_in_logs() {
        let l = reply(r#"{"access_token":"ghu_secret","refresh_token":"ghr_secret"}"#).login("me".into(), 0).unwrap();
        assert!(!format!("{l:?}").contains("secret"));
    }
}
