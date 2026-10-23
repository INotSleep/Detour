//! SSH key material: generate, import, export, passphrases.
//!
//! Private keys are always stored in OpenSSH format. A key with a passphrase is
//! stored encrypted with it, exactly like `ssh-keygen -p` would leave it on disk.

use russh::keys::ssh_key::{HashAlg, LineEnding};
use russh::keys::{Algorithm, PrivateKey};

use crate::model::{KeyAlgorithm, KeyEntry, now_ms};
use crate::{Error, Result, new_id};

/// RSA size for generated keys.
const RSA_BITS: usize = 4096;

fn invalid(reason: impl Into<String>) -> Error {
    Error::InvalidKey { reason: reason.into() }
}

fn algorithm_of(key: &PrivateKey) -> Result<(KeyAlgorithm, Option<u32>)> {
    match key.algorithm() {
        Algorithm::Ed25519 => Ok((KeyAlgorithm::Ed25519, None)),
        Algorithm::Ecdsa { .. } => Ok((KeyAlgorithm::Ecdsa, None)),
        Algorithm::Rsa { .. } => {
            let bits = key.public_key().key_data().rsa().map(|k| k.key_size());
            Ok((KeyAlgorithm::Rsa, bits))
        }
        other => Err(invalid(format!("unsupported algorithm {other}"))),
    }
}

/// Encodes a decrypted key for storage, encrypting it when a passphrase is given.
fn encode(key: &PrivateKey, passphrase: Option<&str>) -> Result<String> {
    let key = match passphrase {
        Some(p) => key.encrypt(&mut rand::rng(), p).map_err(|e| invalid(e.to_string()))?,
        None => key.clone(),
    };
    Ok(key.to_openssh(LineEnding::LF).map_err(|e| invalid(e.to_string()))?.to_string())
}

fn entry(name: &str, key: &PrivateKey, passphrase: Option<&str>, remember: bool) -> Result<KeyEntry> {
    let (algorithm, bits) = algorithm_of(key)?;
    let passphrase = passphrase.filter(|p| !p.is_empty());
    Ok(KeyEntry {
        id: new_id(),
        name: name.to_string(),
        algorithm,
        bits,
        fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        public_key: key.public_key().to_openssh().map_err(|e| invalid(e.to_string()))?,
        private_key: encode(key, passphrase)?,
        has_passphrase: passphrase.is_some(),
        passphrase: passphrase.filter(|_| remember).map(str::to_string),
        created_at: now_ms(),
    })
}

/// Generates a new key. `name` also becomes the key comment.
pub fn generate(name: &str, algorithm: KeyAlgorithm, passphrase: Option<&str>, remember: bool) -> Result<KeyEntry> {
    let mut key = match algorithm {
        KeyAlgorithm::Ed25519 => PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519),
        KeyAlgorithm::Rsa => russh::keys::ssh_key::private::RsaKeypair::random(&mut rand::rng(), RSA_BITS).and_then(|pair| PrivateKey::new(pair.into(), name)),
        KeyAlgorithm::Ecdsa => return Err(invalid("ECDSA generation is not offered")),
    }
    .map_err(|e| invalid(e.to_string()))?;
    key.set_comment(name);
    entry(name, &key, passphrase, remember)
}

/// Does this private key text need a passphrase to be read?
pub fn is_encrypted(text: &str) -> bool {
    match PrivateKey::from_openssh(text.trim()) {
        Ok(k) => k.is_encrypted(),
        // Legacy PEM formats mark encryption in their headers.
        Err(_) => text.contains("ENCRYPTED"),
    }
}

/// Decrypts any private key format russh understands (OpenSSH, PKCS#1, PKCS#8).
fn decode(text: &str, passphrase: Option<&str>) -> Result<PrivateKey> {
    let text = text.trim();
    if text.starts_with("ssh-") || text.starts_with("ecdsa-") {
        return Err(invalid("this is a public key, a private key is needed"));
    }
    if text.contains("PuTTY-User-Key-File") {
        return Err(invalid("PuTTY format: export the key as OpenSSH first"));
    }
    if is_encrypted(text) && passphrase.is_none_or(str::is_empty) {
        return Err(Error::PassphraseRequired);
    }
    russh::keys::decode_secret_key(text, passphrase.filter(|p| !p.is_empty()))
        .map_err(|e| if is_encrypted(text) { Error::WrongPassphrase } else { invalid(e.to_string()) })
}

/// Imports a private key. With a passphrase the key stays encrypted with it in the vault.
pub fn import(name: &str, text: &str, passphrase: Option<&str>, remember: bool) -> Result<KeyEntry> {
    let key = decode(text, passphrase)?;
    let name = if name.trim().is_empty() { key.comment().to_string() } else { name.trim().to_string() };
    let name = if name.is_empty() { "imported-key".to_string() } else { name };
    entry(&name, &key, passphrase, remember)
}

/// Opens a stored key for authentication.
pub fn unlock(entry: &KeyEntry, passphrase: Option<&str>) -> Result<PrivateKey> {
    let passphrase = passphrase.or(entry.passphrase.as_deref());
    decode(&entry.private_key, if entry.has_passphrase { passphrase } else { None })
}

/// Private key file contents for export. An encrypted key is exported as is;
/// an unencrypted one can be protected with `new_passphrase` for the file.
pub fn export(entry: &KeyEntry, new_passphrase: Option<&str>) -> Result<String> {
    match new_passphrase.filter(|p| !p.is_empty()) {
        Some(p) if !entry.has_passphrase => encode(&unlock(entry, None)?, Some(p)),
        _ => Ok(entry.private_key.clone()),
    }
}

/// Re-encrypts a key with a new passphrase (`None` removes it). Returns the updated entry.
pub fn change_passphrase(entry: &KeyEntry, current: Option<&str>, new: Option<&str>, remember: bool) -> Result<KeyEntry> {
    let key = unlock(entry, current)?;
    let new = new.filter(|p| !p.is_empty());
    Ok(KeyEntry {
        private_key: encode(&key, new)?,
        has_passphrase: new.is_some(),
        passphrase: new.filter(|_| remember).map(str::to_string),
        ..entry.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ed25519_roundtrips_through_import() {
        let k = generate("test", KeyAlgorithm::Ed25519, None, false).unwrap();
        assert!(k.public_key.starts_with("ssh-ed25519 "));
        assert!(k.fingerprint.starts_with("SHA256:"));
        assert!(!k.has_passphrase);
        let again = import("", &k.private_key, None, false).unwrap();
        assert_eq!(again.fingerprint, k.fingerprint);
        assert_eq!(again.name, "test");
    }

    #[test]
    fn passphrase_protects_stored_key() {
        let k = generate("p", KeyAlgorithm::Ed25519, Some("hunter2"), false).unwrap();
        assert!(k.has_passphrase);
        assert!(k.passphrase.is_none());
        assert!(is_encrypted(&k.private_key));
        assert!(matches!(unlock(&k, None), Err(Error::PassphraseRequired)));
        assert!(matches!(unlock(&k, Some("nope")), Err(Error::WrongPassphrase)));
        assert!(unlock(&k, Some("hunter2")).is_ok());
    }

    #[test]
    fn remembered_passphrase_unlocks_without_asking() {
        let k = generate("p", KeyAlgorithm::Ed25519, Some("pw"), true).unwrap();
        assert_eq!(k.passphrase.as_deref(), Some("pw"));
        assert!(unlock(&k, None).is_ok());
    }

    #[test]
    fn change_and_remove_passphrase() {
        let k = generate("p", KeyAlgorithm::Ed25519, Some("old"), false).unwrap();
        let k2 = change_passphrase(&k, Some("old"), Some("new"), false).unwrap();
        assert!(unlock(&k2, Some("new")).is_ok());
        let k3 = change_passphrase(&k2, Some("new"), None, false).unwrap();
        assert!(!k3.has_passphrase && !is_encrypted(&k3.private_key));
        assert_eq!(k3.fingerprint, k.fingerprint);
    }

    #[test]
    fn export_protects_plain_key_when_asked() {
        let k = generate("p", KeyAlgorithm::Ed25519, None, false).unwrap();
        assert!(!is_encrypted(&export(&k, None).unwrap()));
        let file = export(&k, Some("file-pw")).unwrap();
        assert!(is_encrypted(&file));
        assert_eq!(import("", &file, Some("file-pw"), false).unwrap().fingerprint, k.fingerprint);
    }

    #[test]
    fn import_rejects_public_key_and_garbage() {
        let k = generate("p", KeyAlgorithm::Ed25519, None, false).unwrap();
        assert!(matches!(import("", &k.public_key, None, false), Err(Error::InvalidKey { .. })));
        assert!(matches!(import("", "hello", None, false), Err(Error::InvalidKey { .. })));
        let enc = generate("p", KeyAlgorithm::Ed25519, Some("x"), false).unwrap();
        assert!(matches!(import("", &enc.private_key, None, false), Err(Error::PassphraseRequired)));
    }
}
