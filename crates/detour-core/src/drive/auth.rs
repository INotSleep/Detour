//! Signing in to a drive with OAuth, as a public client (no secret of Detour's to keep): in
//! the browser with PKCE, coming back to a port this device listens on. (Microsoft's device
//! code flow is not used: personal accounts get a phishing warning and a dead end with it.)
//!
//! Each device signs in itself; the sign-in is kept in the vault and never syncs.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::http::{AGENT, enc, net_error};
use super::{DriveLogin, Provider};
use crate::model::now_ms;
use crate::store::Store;
use crate::{Error, Result};

/* ---------- the apps Detour is registered as ---------- */

/// Detour's app ids, public by design. A build can use its own: `DETOUR_ONEDRIVE_CLIENT_ID`,
/// `DETOUR_GDRIVE_CLIENT_ID` (+ `DETOUR_GDRIVE_CLIENT_SECRET`), `DETOUR_DROPBOX_CLIENT_ID`.
const ONEDRIVE_CLIENT_ID: &str = "02e1bc0d-f5fb-4d4f-82f0-0c8096228794";
const GDRIVE_CLIENT_ID: &str = "37796379108-us7c38abrstv524hb8lb875qiajhuhgu.apps.googleusercontent.com";
/// Google calls it a secret, but for an installed app it is not one (Google says so).
const GDRIVE_CLIENT_SECRET: &str = "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";
const DROPBOX_CLIENT_ID: &str = "07zhut87lxovpwt";
/// Registered with Dropbox as the redirect: it takes exact addresses only.
const DROPBOX_PORT: u16 = 53682;

fn env_or(name: &str, fallback: &str) -> Option<String> {
    let v = std::env::var(name).unwrap_or_else(|_| fallback.to_string());
    (!v.trim().is_empty()).then(|| v.trim().to_string())
}

fn client_id(p: Provider) -> Result<String> {
    let id = match p {
        Provider::OneDrive => env_or("DETOUR_ONEDRIVE_CLIENT_ID", ONEDRIVE_CLIENT_ID),
        Provider::GoogleDrive => env_or("DETOUR_GDRIVE_CLIENT_ID", GDRIVE_CLIENT_ID),
        Provider::Dropbox => env_or("DETOUR_DROPBOX_CLIENT_ID", DROPBOX_CLIENT_ID),
    };
    id.ok_or_else(|| Error::Drive { provider: p.id().into(), message: "this build of Detour has no app id for it".into() })
}

fn ms_login() -> String {
    std::env::var("DETOUR_MS_LOGIN").unwrap_or_else(|_| "https://login.microsoftonline.com/common/oauth2/v2.0".into())
}

const ONEDRIVE_SCOPE: &str = "Files.ReadWrite.All User.Read offline_access";
const GDRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";

fn token_url(p: Provider) -> String {
    match p {
        Provider::OneDrive => format!("{}/token", ms_login()),
        Provider::GoogleDrive => std::env::var("DETOUR_GOOGLE_TOKEN").unwrap_or_else(|_| "https://oauth2.googleapis.com/token".into()),
        Provider::Dropbox => std::env::var("DETOUR_DROPBOX_TOKEN").unwrap_or_else(|_| "https://api.dropboxapi.com/oauth2/token".into()),
    }
}

/* ---------- what the UI is told ---------- */

/// How the person signs in.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Start {
    /// Enter `user_code` at `verification_uri`, then poll.
    Device { user_code: String, verification_uri: String, interval: u64 },
    /// Open `url`; poll until done. With `paste`, the page shows a code to paste back instead.
    Browser { url: String, paste: bool },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Poll {
    Pending,
    SlowDown,
    Done { account: String },
    Expired,
    Denied,
}

/* ---------- sign-ins under way ---------- */

#[derive(Deserialize)]
struct Tokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

enum Pending {
    Browser { verifier: String, redirect: Option<String>, done: Option<std::result::Result<Tokens, Error>> },
}

static PENDING: LazyLock<Mutex<HashMap<Provider, Pending>>> = LazyLock::new(Default::default);

fn pending() -> std::sync::MutexGuard<'static, HashMap<Provider, Pending>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// The listener waiting for the browser, per drive: a new sign-in stops the one before it (whose
/// port Dropbox's fixed one would otherwise still hold).
static WAITING: LazyLock<Mutex<HashMap<Provider, tokio::task::AbortHandle>>> = LazyLock::new(Default::default);

fn stop_waiting(p: Provider) {
    if let Some(h) = WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(&p) {
        h.abort();
    }
}

fn locked(store: &Mutex<Store>) -> Result<std::sync::MutexGuard<'_, Store>> {
    store.lock().map_err(|_| Error::Internal { message: "state lock poisoned".into() })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::Internal { message: e.to_string() })?
}

/// POSTs a form to an OAuth endpoint; refusals come back as JSON too.
fn post_form(p: Provider, url: &str, form: &[(&str, String)]) -> Result<Tokens> {
    let mut resp = AGENT.post(url).header("Accept", "application/json").send_form(form.iter().map(|(k, v)| (*k, v.as_str()))).map_err(|e| net_error(p, e))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().map_err(|e| net_error(p, e))?;
    serde_json::from_str(&body).map_err(|_| Error::Drive { provider: p.id().into(), message: format!("sign-in answered {status}") })
}

fn random() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Starts signing in to a drive. With `back`, the browser is sent there once the drive has
/// answered (a phone's sign-in sheet closes on the app's own link), with `ok=1` or `ok=0`.
pub async fn start(p: Provider, back: Option<String>) -> Result<Start> {
    let client = client_id(p)?;
    match p {
        Provider::OneDrive | Provider::GoogleDrive | Provider::Dropbox => {
            let verifier = random();
            let state = random();
            let port = if p == Provider::Dropbox { DROPBOX_PORT } else { 0 };
            stop_waiting(p);
            let mut listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.ok();
            // The stopped listener lets go of the port once its task is dropped.
            for _ in 0..20 {
                if listener.is_some() || port == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.ok();
            }
            let redirect = match &listener {
                // Microsoft takes loopback addresses by the name it was registered with.
                Some(l) if p == Provider::OneDrive => Some(format!("http://localhost:{}/", l.local_addr()?.port())),
                Some(l) => Some(format!("http://127.0.0.1:{}/", l.local_addr()?.port())),
                None if p == Provider::Dropbox => None,
                None => return Err(Error::Drive { provider: p.id().into(), message: "can't listen for the sign-in".into() }),
            };
            let mut url = match p {
                Provider::OneDrive => format!("{}/authorize?client_id={}&response_type=code&response_mode=query&scope={}&prompt=select_account", ms_login(), enc(&client), enc(ONEDRIVE_SCOPE)),
                Provider::GoogleDrive => format!(
                    "{}?client_id={}&response_type=code&scope={}&access_type=offline&prompt=consent",
                    std::env::var("DETOUR_GOOGLE_AUTH").unwrap_or_else(|_| "https://accounts.google.com/o/oauth2/v2/auth".into()),
                    enc(&client),
                    enc(GDRIVE_SCOPE)
                ),
                _ => format!(
                    "{}?client_id={}&response_type=code&token_access_type=offline",
                    std::env::var("DETOUR_DROPBOX_AUTH").unwrap_or_else(|_| "https://www.dropbox.com/oauth2/authorize".into()),
                    enc(&client)
                ),
            };
            url.push_str(&format!("&code_challenge={}&code_challenge_method=S256", challenge(&verifier)));
            if let Some(r) = &redirect {
                url.push_str(&format!("&redirect_uri={}&state={}", enc(r), enc(&state)));
            }
            pending().insert(p, Pending::Browser { verifier: verifier.clone(), redirect: redirect.clone(), done: None });
            if let (Some(l), Some(r)) = (listener, redirect) {
                let task = tokio::spawn(async move {
                    let got = match tokio::time::timeout(Duration::from_secs(600), wait_for_code(l, &state, back.as_deref())).await {
                        Ok(Ok(code)) => {
                            let client = client.clone();
                            blocking(move || exchange(p, &client, &code, &verifier, Some(&r))).await
                        }
                        Ok(Err(e)) => Err(e),
                        Err(_) => Err(Error::Drive { provider: p.id().into(), message: "the sign-in timed out".into() }),
                    };
                    if let Some(Pending::Browser { done, .. }) = pending().get_mut(&p) {
                        *done = Some(got);
                    }
                });
                WAITING.lock().unwrap_or_else(|e| e.into_inner()).insert(p, task.abort_handle());
            }
            Ok(Start::Browser { url, paste: !matches!(pending().get(&p), Some(Pending::Browser { redirect: Some(_), .. })) })
        }
    }
}

/// Swaps a browser sign-in's code for tokens.
fn exchange(p: Provider, client: &str, code: &str, verifier: &str, redirect: Option<&str>) -> Result<Tokens> {
    let mut form = vec![("client_id", client.to_string()), ("grant_type", "authorization_code".into()), ("code", code.to_string()), ("code_verifier", verifier.to_string())];
    if let Some(r) = redirect {
        form.push(("redirect_uri", r.to_string()));
    }
    if p == Provider::GoogleDrive
        && let Some(s) = env_or("DETOUR_GDRIVE_CLIENT_SECRET", GDRIVE_CLIENT_SECRET)
    {
        form.push(("client_secret", s));
    }
    if p == Provider::OneDrive {
        form.push(("scope", ONEDRIVE_SCOPE.into()));
    }
    let t = post_form(p, &token_url(p), &form)?;
    match &t.access_token {
        Some(_) => Ok(t),
        None => Err(Error::Drive { provider: p.id().into(), message: t.error_description.or(t.error).unwrap_or_else(|| "the sign-in failed".into()) }),
    }
}

/// Waits for the browser to come back with the code (other requests, like a favicon, are turned away).
async fn wait_for_code(l: tokio::net::TcpListener, state: &str, back: Option<&str>) -> Result<String> {
    loop {
        let (mut s, _) = l.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = s.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
        let query: HashMap<String, String> = target.split_once('?').map(|(_, q)| q).unwrap_or("").split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), decode(v))).collect();
        if !query.contains_key("code") && !query.contains_key("error") {
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let ok = query.get("state").map(String::as_str) == Some(state) && query.contains_key("code");
        let page = if ok { "Detour: signed in. You can close this tab. / Вход выполнен, вкладку можно закрыть." } else { "Detour: the sign-in did not go through. / Вход не выполнен." };
        let body = format!("<!doctype html><meta charset=utf-8><title>Detour</title><body style=\"font:16px system-ui;padding:40px\">{page}</body>");
        let head = match back {
            Some(b) => format!("HTTP/1.1 302 Found\r\nLocation: {b}{}ok={}\r\n", if b.contains('?') { '&' } else { '?' }, u8::from(ok)),
            None => "HTTP/1.1 200 OK\r\n".into(),
        };
        let _ = s.write_all(format!("{head}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
        return match (ok, query.get("code")) {
            (true, Some(code)) => Ok(code.clone()),
            _ => Err(Error::Drive { provider: "oauth".into(), message: query.get("error").cloned().unwrap_or_else(|| "the sign-in was refused".into()) }),
        };
    }
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Has the person finished signing in? Once they have, the sign-in is kept on this device.
pub async fn poll(store: &Mutex<Store>, p: Provider) -> Result<Poll> {
    // Taken out under the lock, which must not be held across the network calls below.
    let done = {
        let mut map = pending();
        match map.get_mut(&p) {
            None => return Ok(Poll::Expired),
            Some(Pending::Browser { done, .. }) => {
                let d = done.take();
                if d.is_some() {
                    map.remove(&p);
                }
                d
            }
        }
    };
    let tokens = match done {
        None => return Ok(Poll::Pending),
        Some(r) => r?,
    };
    pending().remove(&p);
    let account = keep(store, p, tokens).await?;
    Ok(Poll::Done { account })
}

/// Finishes a browser sign-in whose page showed a code to paste (Dropbox, when its port was taken).
pub async fn finish_code(store: &Mutex<Store>, p: Provider, code: &str) -> Result<String> {
    let client = client_id(p)?;
    let verifier = match pending().get(&p) {
        Some(Pending::Browser { verifier, redirect: None, .. }) => verifier.clone(),
        _ => return Err(Error::Drive { provider: p.id().into(), message: "start signing in again".into() }),
    };
    let code = code.trim().to_string();
    let tokens = blocking(move || exchange(p, &client, &code, &verifier, None)).await?;
    pending().remove(&p);
    keep(store, p, tokens).await
}

/// Keeps a fresh sign-in, with the account it is for.
async fn keep(store: &Mutex<Store>, p: Provider, t: Tokens) -> Result<String> {
    let token = t.access_token.clone().unwrap_or_default();
    let tok = token.clone();
    let account = blocking(move || match p {
        Provider::OneDrive => super::onedrive::account(&tok),
        Provider::GoogleDrive => super::gdrive::account(&tok),
        Provider::Dropbox => super::dropbox::account(&tok),
    })
    .await?;
    let login = DriveLogin { provider: p, account: account.clone(), access_token: token, expires_at: now_ms() + t.expires_in.unwrap_or(3600) * 1000, refresh_token: t.refresh_token };
    locked(store)?.set_drive_login(login)?;
    Ok(account)
}

/// Only one renewal at a time per drive: some replace the refresh token each time.
static RENEWING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

/// A token to call the drive with, renewed first when it is about to run out.
pub async fn token(store: &Mutex<Store>, p: Provider) -> Result<String> {
    let _held = RENEWING.lock().await;
    let login = locked(store)?.drive_login(p)?.ok_or_else(|| Error::DriveSignIn { provider: p.id().into() })?;
    if login.access_token.is_empty() {
        return Err(Error::DriveSignIn { provider: p.id().into() });
    }
    if login.expires_at - 5 * 60 * 1000 > now_ms() {
        return Ok(login.access_token);
    }
    let Some(refresh) = login.refresh_token.clone() else { return Err(Error::DriveSignIn { provider: p.id().into() }) };
    let client = client_id(p)?;
    let mut form = vec![("client_id", client), ("grant_type", "refresh_token".to_string()), ("refresh_token", refresh.clone())];
    match p {
        Provider::OneDrive => form.push(("scope", ONEDRIVE_SCOPE.into())),
        Provider::GoogleDrive => {
            if let Some(s) = env_or("DETOUR_GDRIVE_CLIENT_SECRET", GDRIVE_CLIENT_SECRET) {
                form.push(("client_secret", s));
            }
        }
        Provider::Dropbox => {}
    }
    match blocking(move || post_form(p, &token_url(p), &form)).await {
        Ok(Tokens { access_token: Some(access), refresh_token, expires_in, .. }) => {
            let renewed = DriveLogin { access_token: access.clone(), expires_at: now_ms() + expires_in.unwrap_or(3600) * 1000, refresh_token: refresh_token.or(Some(refresh)), ..login };
            locked(store)?.set_drive_login(renewed)?;
            Ok(access)
        }
        // Refused: the sign-in is over (revoked, or unused for too long).
        Ok(_) => {
            locked(store)?.set_drive_login(DriveLogin { access_token: String::new(), ..login })?;
            Err(Error::DriveSignIn { provider: p.id().into() })
        }
        // Offline: the old token may still work for a while.
        Err(e) if login.expires_at > now_ms() => {
            let _ = e;
            Ok(login.access_token)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_rfc7636() {
        // RFC 7636, appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(decode("4%2F0Ab_c%3D+x"), "4/0Ab_c= x");
        assert_eq!(decode("50%"), "50%");
    }
}
