//! Server-side TLS (SPEC §14.3) using rustls with the `ring` provider, which
//! builds on Windows/MSVC without CMake or NASM.
//!
//! Two modes besides `off`:
//! - `files`: a PEM certificate chain and private key supplied by the operator.
//! - `self_signed`: generated on first run into `<data_dir>/tls/` and reused. The
//!   SHA-256 fingerprint is logged so users can verify it in the browser.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

use crate::config::{Config, TlsMode};

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no certificates found in {0}")]
    NoCertificates(PathBuf),
    #[error("no private key found in {0}")]
    NoPrivateKey(PathBuf),
    #[error("invalid PEM in {path}: {message}")]
    Pem { path: PathBuf, message: String },
    #[error("certificate generation failed: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("invalid TLS configuration: {0}")]
    Rustls(#[from] rustls::Error),
}

/// A ready-to-serve TLS configuration plus details worth showing the operator.
pub struct TlsSetup {
    pub config: Arc<rustls::ServerConfig>,
    /// SHA-256 of the leaf certificate, colon-separated uppercase hex.
    pub fingerprint_sha256: String,
    pub cert_path: PathBuf,
    pub generated: bool,
}

impl std::fmt::Debug for TlsSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsSetup")
            .field("fingerprint_sha256", &self.fingerprint_sha256)
            .field("cert_path", &self.cert_path)
            .field("generated", &self.generated)
            .finish_non_exhaustive()
    }
}

/// Build the TLS configuration for `config`, or `None` when TLS is off.
pub fn load(config: &Config) -> Result<Option<TlsSetup>, TlsError> {
    let tls = &config.server.tls;
    match tls.mode {
        TlsMode::Off => Ok(None),
        TlsMode::Files => {
            // `Config::validate` guarantees both paths are set in this mode.
            let (Some(cert), Some(key)) = (&tls.cert_path, &tls.key_path) else {
                return Err(TlsError::Pem {
                    path: PathBuf::new(),
                    message: "cert_path and key_path are required".into(),
                });
            };
            from_files(cert, key, false).map(Some)
        }
        TlsMode::SelfSigned => {
            let dir = config.data_dir.join("tls");
            let cert = dir.join("self-signed.crt");
            let key = dir.join("self-signed.key");
            let generated = if cert.is_file() && key.is_file() {
                false
            } else {
                generate_self_signed(&dir, &cert, &key, &tls.self_signed_names)?;
                true
            };
            from_files(&cert, &key, generated).map(Some)
        }
    }
}

fn read(path: &Path) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|source| TlsError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn from_files(cert_path: &Path, key_path: &Path, generated: bool) -> Result<TlsSetup, TlsError> {
    let cert_pem = read(cert_path)?;
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| TlsError::Pem {
            path: cert_path.to_path_buf(),
            message: e.to_string(),
        })?;
    let leaf = certs
        .first()
        .ok_or_else(|| TlsError::NoCertificates(cert_path.to_path_buf()))?;
    let fingerprint_sha256 = fingerprint(leaf);

    let key_pem = read(key_path)?;
    let key = match PrivateKeyDer::from_pem_slice(&key_pem) {
        Ok(key) => key,
        Err(rustls::pki_types::pem::Error::NoItemsFound) => {
            return Err(TlsError::NoPrivateKey(key_path.to_path_buf()))
        }
        Err(e) => {
            return Err(TlsError::Pem {
                path: key_path.to_path_buf(),
                message: e.to_string(),
            })
        }
    };

    let mut server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(certs, key)?;
    server.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(TlsSetup {
        config: Arc::new(server),
        fingerprint_sha256,
        cert_path: cert_path.to_path_buf(),
        generated,
    })
}

fn generate_self_signed(
    dir: &Path,
    cert_path: &Path,
    key_path: &Path,
    extra_names: &[String],
) -> Result<(), TlsError> {
    std::fs::create_dir_all(dir).map_err(|source| TlsError::Write {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut names = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    if let Some(host) = hostname() {
        names.push(host);
    }
    names.extend(extra_names.iter().cloned());
    names.dedup();

    let certified = rcgen::generate_simple_self_signed(names)?;
    write_private(key_path, certified.signing_key.serialize_pem().as_bytes())?;
    std::fs::write(cert_path, certified.cert.pem()).map_err(|source| TlsError::Write {
        path: cert_path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Write a private key readable only by the current user where the OS supports
/// it. On Windows the file inherits the data directory's ACL; the default
/// `%ProgramData%\CraneCamera` should be restricted to administrators and the
/// service account (documented in docs/development.md).
fn write_private(path: &Path, contents: &[u8]) -> Result<(), TlsError> {
    let err = |source| TlsError::Write {
        path: path.to_path_buf(),
        source,
    };
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(err)?;
        file.write_all(contents).map_err(err)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents).map_err(err)
    }
}

fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty())
}

pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    Sha256::digest(cert.as_ref())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_in(dir: &Path, mode: TlsMode) -> Config {
        let mut config = Config {
            data_dir: dir.to_path_buf(),
            ..Config::default()
        };
        config.server.tls.mode = mode;
        config
    }

    #[test]
    fn off_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&config_in(dir.path(), TlsMode::Off))
            .unwrap()
            .is_none());
    }

    #[test]
    fn self_signed_is_generated_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path(), TlsMode::SelfSigned);
        let first = load(&config).unwrap().unwrap();
        assert!(first.generated);
        assert_eq!(first.fingerprint_sha256.len(), 32 * 3 - 1);
        let second = load(&config).unwrap().unwrap();
        assert!(!second.generated);
        assert_eq!(first.fingerprint_sha256, second.fingerprint_sha256);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("tls/self-signed.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn files_mode_loads_provided_pem() {
        let dir = tempfile::tempdir().unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["crane.test".into()]).unwrap();
        let cert = dir.path().join("c.pem");
        let key = dir.path().join("k.pem");
        std::fs::write(&cert, certified.cert.pem()).unwrap();
        std::fs::write(&key, certified.signing_key.serialize_pem()).unwrap();

        let mut config = config_in(dir.path(), TlsMode::Files);
        config.server.tls.cert_path = Some(cert);
        config.server.tls.key_path = Some(key);
        let setup = load(&config).unwrap().unwrap();
        assert!(!setup.generated);
        assert_eq!(setup.fingerprint_sha256, fingerprint(certified.cert.der()));
    }

    #[test]
    fn files_mode_reports_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["crane.test".into()]).unwrap();
        let cert = dir.path().join("c.pem");
        std::fs::write(&cert, certified.cert.pem()).unwrap();
        let mut config = config_in(dir.path(), TlsMode::Files);
        config.server.tls.cert_path = Some(cert.clone());
        config.server.tls.key_path = Some(cert); // a cert, not a key
        assert!(matches!(load(&config), Err(TlsError::NoPrivateKey(_))));
    }
}
