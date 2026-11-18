//! Per-camera TLS verification (SPEC §4.5): strict (operating-system trust
//! store), pinned (exact certificate fingerprint) or insecure-skip-verify.
//!
//! Every mode records the SHA-256 fingerprint of the certificate the camera
//! presented, so "test connection" can show it and an admin can pin it.
//! Handshake signatures are always verified, even in insecure mode: that mode
//! only skips *certificate* validation.

use std::fmt;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

/// How to validate a camera's certificate.
#[derive(Clone, PartialEq, Eq)]
pub enum TlsPolicy {
    Strict,
    /// Uppercase, colon-separated SHA-256 fingerprint.
    Pinned(String),
    InsecureSkipVerify,
}

impl fmt::Debug for TlsPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TlsPolicy::Strict => f.write_str("Strict"),
            TlsPolicy::Pinned(_) => f.write_str("Pinned"),
            TlsPolicy::InsecureSkipVerify => f.write_str("InsecureSkipVerify"),
        }
    }
}

/// The fingerprint of the last certificate seen on this connection.
pub type SeenFingerprint = Arc<Mutex<Option<String>>>;

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[derive(Debug)]
struct CameraVerifier {
    policy: TlsPolicy,
    /// Present for `Strict`.
    platform: Option<rustls_platform_verifier::Verifier>,
    provider: Arc<CryptoProvider>,
    seen: SeenFingerprint,
}

impl ServerCertVerifier for CameraVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let fingerprint = crate::tls::fingerprint(end_entity);
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fingerprint.clone());
        match &self.policy {
            TlsPolicy::Strict => match &self.platform {
                Some(v) => {
                    v.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
                }
                None => Err(TlsError::General(
                    "platform certificate verifier unavailable".into(),
                )),
            },
            TlsPolicy::Pinned(expected) if *expected == fingerprint => {
                Ok(ServerCertVerified::assertion())
            }
            TlsPolicy::Pinned(_) => Err(TlsError::General(
                "certificate does not match the pinned fingerprint".into(),
            )),
            TlsPolicy::InsecureSkipVerify => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Build a rustls client config for `policy`, returning where the presented
/// certificate's fingerprint will be recorded.
pub fn client_config(
    policy: &TlsPolicy,
) -> Result<(rustls::ClientConfig, SeenFingerprint), TlsError> {
    let provider = provider();
    let platform = match policy {
        TlsPolicy::Strict => Some(rustls_platform_verifier::Verifier::new(provider.clone())?),
        _ => None,
    };
    let seen = SeenFingerprint::default();
    let verifier = Arc::new(CameraVerifier {
        policy: policy.clone(),
        platform,
        provider: provider.clone(),
        seen: seen.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok((config, seen))
}
