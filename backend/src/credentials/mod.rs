//! Camera credential storage (SPEC §4.6, ADR 0003).
//!
//! Credentials are encrypted by a [`CredentialStore`] before they reach SQLite;
//! the database only ever holds ciphertext. Each ciphertext starts with a short
//! tag naming the backend (`dpapi1:` / `aesgcm1:`), so a database moved to a
//! machine with a different backend fails loudly instead of returning garbage.
//!
//! - Windows: DPAPI (`CryptProtectData`) with machine scope and app-specific
//!   entropy, so it keeps working if the app later runs as a Windows service.
//! - Elsewhere (development and CI only): AES-256-GCM with a random key in
//!   `<data_dir>/keys/credentials.key`. A warning is logged at startup.

#[cfg(windows)]
mod dpapi;
mod repo;

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use repo::*;

use crate::secret::{ExposeSecret, SecretString};

/// Username and password for a camera. The password never appears in `Debug`.
#[derive(Clone, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: SecretString,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// The serialized plaintext. Kept private: only the store sees it.
#[derive(Serialize, Deserialize)]
struct Plain<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed (wrong machine, wrong key, or corrupted data)")]
    Decrypt,
    #[error("ciphertext was produced by a different credential store ({0})")]
    WrongBackend(String),
    #[error("credential key file error at {path}: {source}")]
    KeyFile {
        path: String,
        source: std::io::Error,
    },
}

/// Encrypts and decrypts credential blobs.
pub trait CredentialStore: Send + Sync {
    /// A short name for logs and errors, e.g. `dpapi` or `aes-gcm (dev only)`.
    fn name(&self) -> &'static str;
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CredentialError>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CredentialError>;

    /// Encrypt credentials for storage.
    fn seal(&self, creds: &Credentials) -> Result<Vec<u8>, CredentialError> {
        let plain = serde_json::to_vec(&Plain {
            username: &creds.username,
            password: creds.password.expose_secret(),
        })
        .map_err(|e| CredentialError::Encrypt(e.to_string()))?;
        let out = self.encrypt(&plain);
        drop(SecretString::from(
            String::from_utf8(plain).unwrap_or_default(),
        ));
        out
    }

    /// Decrypt stored credentials.
    fn open(&self, ciphertext: &[u8]) -> Result<Credentials, CredentialError> {
        let plain = self.decrypt(ciphertext)?;
        let parsed: Plain<'_> =
            serde_json::from_slice(&plain).map_err(|_| CredentialError::Decrypt)?;
        let creds = Credentials {
            username: parsed.username.to_owned(),
            password: SecretString::from(parsed.password.to_owned()),
        };
        // Overwrite the decrypted buffer by moving it into a zeroizing type.
        drop(SecretString::from(
            String::from_utf8(plain).unwrap_or_default(),
        ));
        Ok(creds)
    }
}

/// App-specific entropy / associated data: ciphertext from another application
/// using the same backend can't be decrypted as ours.
const CONTEXT: &[u8] = b"CCANH camera credentials v1";

/// Build the store for this platform.
pub fn platform_store(data_dir: &Path) -> Result<Arc<dyn CredentialStore>, CredentialError> {
    #[cfg(windows)]
    {
        let _ = data_dir;
        Ok(Arc::new(dpapi::DpapiStore))
    }
    #[cfg(not(windows))]
    {
        let store = AesGcmStore::open_or_create(&data_dir.join("keys").join("credentials.key"))?;
        tracing::warn!(
            "camera credentials are encrypted with a local key file (AES-GCM); \
             this is a development store, not a production credential store"
        );
        Ok(Arc::new(store))
    }
}

/// AES-256-GCM with a key file. Development / CI only.
pub struct AesGcmStore {
    cipher: aes_gcm::Aes256Gcm,
}

const AES_TAG: &[u8] = b"aesgcm1:";

impl AesGcmStore {
    pub fn open_or_create(path: &Path) -> Result<Self, CredentialError> {
        use aes_gcm::aead::KeyInit;
        let io = |source| CredentialError::KeyFile {
            path: path.display().to_string(),
            source,
        };
        let key = if path.is_file() {
            std::fs::read(path).map_err(io)?
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(io)?;
            }
            let mut key = vec![0u8; 32];
            getrandom::fill(&mut key).map_err(|e| CredentialError::Encrypt(e.to_string()))?;
            write_private(path, &key).map_err(io)?;
            key
        };
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).map_err(|_| {
            io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "key file must contain exactly 32 bytes",
            ))
        })?;
        Ok(Self { cipher })
    }

    /// An in-memory store with a random key, for tests.
    pub fn ephemeral() -> Self {
        use aes_gcm::aead::KeyInit;
        let mut key = [0u8; 32];
        let _ = getrandom::fill(&mut key);
        Self {
            // A 32-byte key is always valid for AES-256.
            cipher: aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap_or_else(|_| unreachable!()),
        }
    }
}

impl CredentialStore for AesGcmStore {
    fn name(&self) -> &'static str {
        "aes-gcm (development only)"
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CredentialError> {
        use aes_gcm::aead::{Aead, Payload};
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| CredentialError::Encrypt(e.to_string()))?;
        let ct = self
            .cipher
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plaintext,
                    aad: CONTEXT,
                },
            )
            .map_err(|e| CredentialError::Encrypt(e.to_string()))?;
        Ok([AES_TAG, &nonce, &ct].concat())
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CredentialError> {
        use aes_gcm::aead::{Aead, Payload};
        let rest = ciphertext
            .strip_prefix(AES_TAG)
            .ok_or_else(|| CredentialError::WrongBackend(backend_tag(ciphertext)))?;
        if rest.len() < 12 {
            return Err(CredentialError::Decrypt);
        }
        let (nonce, ct) = rest.split_at(12);
        let nonce: [u8; 12] = nonce.try_into().map_err(|_| CredentialError::Decrypt)?;
        self.cipher
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: ct,
                    aad: CONTEXT,
                },
            )
            .map_err(|_| CredentialError::Decrypt)
    }
}

fn backend_tag(ciphertext: &[u8]) -> String {
    let end = ciphertext
        .iter()
        .position(|b| *b == b':')
        .unwrap_or(0)
        .min(16);
    String::from_utf8_lossy(&ciphertext[..end]).into_owned()
}

fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials {
            username: "root".into(),
            password: SecretString::from("hunter2-camera-pass".to_owned()),
        }
    }

    fn roundtrip(store: &dyn CredentialStore) {
        let sealed = store.seal(&creds()).unwrap();
        assert!(
            !sealed.windows(19).any(|w| w == b"hunter2-camera-pass"),
            "ciphertext must not contain the password"
        );
        let opened = store.open(&sealed).unwrap();
        assert_eq!(opened.username, "root");
        assert_eq!(opened.password.expose_secret(), "hunter2-camera-pass");
        // Fresh nonce each time.
        assert_ne!(sealed, store.seal(&creds()).unwrap());
    }

    #[test]
    fn aes_gcm_roundtrip_and_tamper_detection() {
        let store: Box<dyn CredentialStore> = Box::new(AesGcmStore::ephemeral());
        roundtrip(&*store);
        let mut sealed = store.seal(&creds()).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(matches!(store.open(&sealed), Err(CredentialError::Decrypt)));
    }

    #[test]
    fn key_file_is_created_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys/credentials.key");
        let a: Box<dyn CredentialStore> = Box::new(AesGcmStore::open_or_create(&path).unwrap());
        let sealed = a.seal(&creds()).unwrap();
        let b: Box<dyn CredentialStore> = Box::new(AesGcmStore::open_or_create(&path).unwrap());
        assert_eq!(b.open(&sealed).unwrap().username, "root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn other_keys_and_backends_are_rejected() {
        let a: Box<dyn CredentialStore> = Box::new(AesGcmStore::ephemeral());
        let b: Box<dyn CredentialStore> = Box::new(AesGcmStore::ephemeral());
        let sealed = a.seal(&creds()).unwrap();
        assert!(matches!(b.open(&sealed), Err(CredentialError::Decrypt)));
        assert!(matches!(
            a.open(b"dpapi1:whatever"),
            Err(CredentialError::WrongBackend(tag)) if tag == "dpapi1"
        ));
    }

    #[test]
    fn debug_redacts_password() {
        let s = format!("{:?}", creds());
        assert!(s.contains("root"));
        assert!(!s.contains("hunter2"));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip() {
        let store: Box<dyn CredentialStore> = Box::new(dpapi::DpapiStore);
        roundtrip(&*store);
        let mut sealed = store.seal(&creds()).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(store.open(&sealed).is_err());
    }
}
