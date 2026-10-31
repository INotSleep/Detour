//! ssh-agent: the system's, used to log in, and the one a server reaches
//! through a forwarded connection (`ssh -A`).
//!
//! A forwarded agent offers the key this login used, if any, and the keys of
//! the system's agent when one runs. It only lists keys and signs: requests to
//! add, remove or lock keys are refused, so the server can't change what is offered.

use std::sync::Arc;

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::ssh_encoding::Encode;
use russh::keys::ssh_key::private::KeypairData;
use russh::keys::ssh_key::Signature;
use russh::keys::{HashAlg, PrivateKey, PublicKey, signature};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Error, Result};

pub type SystemAgent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

/// The system's agent: OpenSSH's (or Pageant) on Windows, `SSH_AUTH_SOCK` elsewhere.
pub async fn system() -> Result<SystemAgent> {
    #[cfg(windows)]
    let agent = match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(a) => a.dynamic(),
        Err(_) => AgentClient::connect_pageant().await.map_err(|_| Error::AgentUnavailable)?.dynamic(),
    };
    #[cfg(not(windows))]
    let agent = AgentClient::connect_env().await.map_err(|_| Error::AgentUnavailable)?.dynamic();
    Ok(agent)
}

/// What a forwarded agent offers besides the system's keys.
#[derive(Clone, Default)]
pub struct Forwarded {
    /// The key the login used, with its name.
    pub own: Option<(Arc<PrivateKey>, String)>,
}

const FAILURE: u8 = 5;
const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const SIGN_REQUEST: u8 = 13;
const SIGN_RESPONSE: u8 = 14;
const RSA_SHA2_256: u32 = 2;
const RSA_SHA2_512: u32 = 4;
const MAX_FRAME: usize = 256 * 1024;

/// Answers agent requests on one forwarded connection until it closes.
pub async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, keys: Forwarded) {
    // Connected on the first request that needs it, and kept for the rest.
    let mut system: Option<SystemAgent> = None;
    loop {
        let mut len = [0u8; 4];
        if stream.read_exact(&mut len).await.is_err() {
            return;
        }
        let len = u32::from_be_bytes(len) as usize;
        if len == 0 || len > MAX_FRAME {
            return;
        }
        let mut frame = vec![0u8; len];
        if stream.read_exact(&mut frame).await.is_err() {
            return;
        }
        let reply = respond(&frame, &keys, &mut system).await.unwrap_or_else(|| vec![FAILURE]);
        let mut out = (reply.len() as u32).to_be_bytes().to_vec();
        out.extend(reply);
        if stream.write_all(&out).await.is_err() || stream.flush().await.is_err() {
            return;
        }
    }
}

async fn respond(frame: &[u8], keys: &Forwarded, system: &mut Option<SystemAgent>) -> Option<Vec<u8>> {
    let (&kind, mut body) = frame.split_first()?;
    let own_blob = keys.own.as_ref().and_then(|(k, _)| k.public_key().to_bytes().ok());
    match kind {
        REQUEST_IDENTITIES => {
            let mut ids: Vec<(Vec<u8>, String)> = Vec::new();
            if let (Some(blob), Some((_, name))) = (&own_blob, &keys.own) {
                ids.push((blob.clone(), name.clone()));
            }
            if let Some(agent) = connected(system).await {
                for id in agent.request_identities().await.unwrap_or_default() {
                    if let AgentIdentity::PublicKey { key, comment } = id
                        && let Ok(blob) = key.to_bytes()
                        && !ids.iter().any(|(b, _)| *b == blob)
                    {
                        ids.push((blob, comment));
                    }
                }
            }
            let mut out = vec![IDENTITIES_ANSWER];
            out.extend((ids.len() as u32).to_be_bytes());
            for (blob, comment) in ids {
                put_string(&mut out, &blob);
                put_string(&mut out, comment.as_bytes());
            }
            Some(out)
        }
        SIGN_REQUEST => {
            let blob = take_string(&mut body)?;
            let data = take_string(&mut body)?;
            let flags = take_u32(&mut body).unwrap_or(0);
            let hash = if flags & RSA_SHA2_512 != 0 {
                Some(HashAlg::Sha512)
            } else if flags & RSA_SHA2_256 != 0 {
                Some(HashAlg::Sha256)
            } else {
                None
            };
            let sig = match &keys.own {
                Some((key, _)) if own_blob.as_deref() == Some(blob) => sign(key, hash, data)?,
                _ => {
                    let public = PublicKey::from_bytes(blob).ok()?;
                    connected(system).await?.sign_request_signature(&public, hash, data).await.ok()?
                }
            };
            let mut out = vec![SIGN_RESPONSE];
            put_string(&mut out, &sig.encode_vec().ok()?);
            Some(out)
        }
        _ => None,
    }
}

async fn connected(system: &mut Option<SystemAgent>) -> Option<&mut SystemAgent> {
    if system.is_none() {
        *system = self::system().await.ok();
    }
    system.as_mut()
}

/// Signs as an agent does: RSA with the hash the request asks for, other keys with their own.
fn sign(key: &PrivateKey, hash: Option<HashAlg>, data: &[u8]) -> Option<Signature> {
    match key.key_data() {
        KeypairData::Rsa(rsa) => signature::Signer::try_sign(&(rsa, hash), data).ok(),
        other => signature::Signer::try_sign(other, data).ok(),
    }
}

fn take_u32(buf: &mut &[u8]) -> Option<u32> {
    let (head, rest) = buf.split_first_chunk::<4>()?;
    *buf = rest;
    Some(u32::from_be_bytes(*head))
}

fn take_string<'a>(buf: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = take_u32(buf)? as usize;
    if buf.len() < len {
        return None;
    }
    let (s, rest) = buf.split_at(len);
    *buf = rest;
    Some(s)
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend((s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ask(keys: &Forwarded, frame: Vec<u8>) -> Vec<u8> {
        let (mut ours, theirs) = tokio::io::duplex(64 * 1024);
        let keys = keys.clone();
        tokio::spawn(serve(theirs, keys));
        let mut out = (frame.len() as u32).to_be_bytes().to_vec();
        out.extend(frame);
        ours.write_all(&out).await.unwrap();
        let mut len = [0u8; 4];
        ours.read_exact(&mut len).await.unwrap();
        let mut reply = vec![0u8; u32::from_be_bytes(len) as usize];
        ours.read_exact(&mut reply).await.unwrap();
        reply
    }

    fn own() -> (Forwarded, PrivateKey) {
        let entry = crate::keys::generate("deploy", crate::model::KeyAlgorithm::Ed25519, None, false).unwrap();
        let key = crate::keys::unlock(&entry, None).unwrap();
        (Forwarded { own: Some((Arc::new(key.clone()), "deploy".into())) }, key)
    }

    #[tokio::test]
    async fn lists_and_signs_with_the_login_key() {
        let (keys, key) = own();
        let blob = key.public_key().to_bytes().unwrap();

        let list = ask(&keys, vec![REQUEST_IDENTITIES]).await;
        assert_eq!(list[0], IDENTITIES_ANSWER);
        let mut body = &list[1..];
        assert!(take_u32(&mut body).unwrap() >= 1);
        assert_eq!(take_string(&mut body).unwrap(), &blob[..]);
        assert_eq!(take_string(&mut body).unwrap(), b"deploy");

        let mut req = vec![SIGN_REQUEST];
        put_string(&mut req, &blob);
        put_string(&mut req, b"challenge");
        req.extend(0u32.to_be_bytes());
        let reply = ask(&keys, req).await;
        assert_eq!(reply[0], SIGN_RESPONSE);
        let mut body = &reply[1..];
        let mut sig_bytes = take_string(&mut body).unwrap();
        let sig = <Signature as russh::keys::ssh_encoding::Decode>::decode(&mut sig_bytes).unwrap();
        assert!(signature::Verifier::verify(key.public_key().key_data(), b"challenge", &sig).is_ok());
    }

    #[tokio::test]
    async fn refuses_changes() {
        let (keys, _) = own();
        // SSH_AGENTC_REMOVE_ALL_IDENTITIES
        assert_eq!(ask(&keys, vec![19]).await, vec![FAILURE]);
    }
}
