//! TLS to a proxy: the system's trusted certificates, or a certificate pinned by its SHA-256.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt;
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use super::failed;
use crate::Result;

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// `pcs`: SHA-256 of certificates, hex (colons allowed), comma separated.
pub fn pins(text: &str) -> Result<Vec<[u8; 32]>> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let hex: String = s.chars().filter(|c| *c != ':').collect();
            let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect::<Option<_>>().ok_or_else(|| failed("protocol"))?;
            <[u8; 32]>::try_from(bytes).map_err(|_| failed("protocol"))
        })
        .collect()
}

/// Accepts the server whose certificate is pinned; any other is checked as usual.
#[derive(Debug)]
struct Pinned {
    pins: Vec<[u8; 32]>,
    usual: Arc<dyn ServerCertVerifier>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, end: &CertificateDer<'_>, others: &[CertificateDer<'_>], name: &ServerName<'_>, ocsp: &[u8], now: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let hash: [u8; 32] = Sha256::digest(end.as_ref()).into();
        if self.pins.contains(&hash) {
            return Ok(ServerCertVerified::assertion());
        }
        self.usual.verify_server_cert(end, others, name, ocsp, now)
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// TLS over `tcp` to the server named `sni`.
pub async fn connect(tcp: TcpStream, sni: &str, alpn: &[String], pinned: &str) -> Result<TlsStream<TcpStream>> {
    let p = provider();
    let builder = ClientConfig::builder_with_provider(p.clone()).with_safe_default_protocol_versions().map_err(|_| failed("tls"))?;
    let pins = pins(pinned)?;
    let mut config = if pins.is_empty() {
        builder.with_platform_verifier().map_err(|_| failed("tls"))?.with_no_client_auth()
    } else {
        let usual = Arc::new(rustls_platform_verifier::Verifier::new(p.clone()).map_err(|_| failed("tls"))?);
        builder.dangerous().with_custom_certificate_verifier(Arc::new(Pinned { pins, usual, provider: p })).with_no_client_auth()
    };
    config.alpn_protocols = alpn.iter().map(|a| a.as_bytes().to_vec()).collect();
    let name = ServerName::try_from(sni.trim_start_matches('[').trim_end_matches(']').to_string()).map_err(|_| failed("tls"))?;
    TlsConnector::from(Arc::new(config)).connect(name, tcp).await.map_err(|e| {
        // A certificate that isn't trusted is "tls"; anything else is the proxy's doing.
        if e.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()).is_some_and(|e| matches!(e, rustls::Error::InvalidCertificate(_))) { failed("tls") } else { failed("protocol") }
    })
}
