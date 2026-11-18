//! Argon2id password hashing.
//!
//! Hashing is CPU-heavy by design (~tens of ms), so callers on the async runtime
//! use the `*_blocking` helpers, which run on Tokio's blocking pool.

use std::sync::OnceLock;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;

use crate::secret::{ExposeSecret, SecretString};

pub const MIN_PASSWORD_LEN: usize = 12;
pub const MAX_PASSWORD_LEN: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password must be at least {MIN_PASSWORD_LEN} characters")]
    TooShort,
    #[error("password must be at most {MAX_PASSWORD_LEN} characters")]
    TooLong,
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("background task failed: {0}")]
    Join(String),
}

/// Check the password policy. Length only, per NIST SP 800-63B: no composition
/// rules, but a minimum length and a sane maximum to bound hashing cost.
pub fn validate(password: &SecretString) -> Result<(), PasswordError> {
    let len = password.expose_secret().chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(PasswordError::TooShort);
    }
    if len > MAX_PASSWORD_LEN {
        return Err(PasswordError::TooLong);
    }
    Ok(())
}

/// Argon2id v19 with the crate defaults (m = 19 MiB, t = 2, p = 1), which match
/// the OWASP minimum recommendation.
fn hasher() -> Argon2<'static> {
    Argon2::default()
}

/// Hash `password` into a PHC string with a random salt.
pub fn hash(password: &SecretString) -> Result<String, PasswordError> {
    hasher()
        .hash_password(password.expose_secret().as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hash(e.to_string()))
}

/// Verify `password` against a stored PHC string. Malformed hashes never verify.
pub fn verify(password: &SecretString, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => hasher()
            .verify_password(password.expose_secret().as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// A valid hash of a random password, verified against when the username doesn't
/// exist so failed logins take the same time either way (no user enumeration).
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let mut random = [0u8; 32];
        // If the OS RNG fails we still produce a valid hash; it only needs to be
        // a hash nobody knows the password for, and timing is what matters.
        let _ = getrandom::fill(&mut random);
        let password = SecretString::from(crate::auth::encode_token(&random));
        hash(&password).unwrap_or_default()
    })
}

pub async fn hash_blocking(password: SecretString) -> Result<String, PasswordError> {
    tokio::task::spawn_blocking(move || hash(&password))
        .await
        .map_err(|e| PasswordError::Join(e.to_string()))?
}

/// Verify on the blocking pool. With `stored = None`, verifies against the dummy
/// hash and always returns `false`.
pub async fn verify_blocking(
    password: SecretString,
    stored: Option<String>,
) -> Result<bool, PasswordError> {
    tokio::task::spawn_blocking(move || match stored {
        Some(stored) => verify(&password, &stored),
        None => {
            let _ = verify(&password, dummy_hash());
            false
        }
    })
    .await
    .map_err(|e| PasswordError::Join(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> SecretString {
        SecretString::from(v.to_owned())
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let stored = hash(&s("correct horse battery")).unwrap();
        assert!(stored.starts_with("$argon2id$v=19$"));
        assert!(verify(&s("correct horse battery"), &stored));
        assert!(!verify(&s("wrong horse battery"), &stored));
    }

    #[test]
    fn same_password_gets_different_salts() {
        assert_ne!(
            hash(&s("correct horse battery")).unwrap(),
            hash(&s("correct horse battery")).unwrap()
        );
    }

    #[test]
    fn malformed_hash_never_verifies() {
        assert!(!verify(&s("anything at all"), "not-a-phc-string"));
        assert!(!verify(&s("anything at all"), ""));
    }

    #[test]
    fn policy_enforces_length() {
        assert!(matches!(
            validate(&s("short")),
            Err(PasswordError::TooShort)
        ));
        assert!(validate(&s("twelve chars")).is_ok());
        assert!(matches!(
            validate(&s(&"x".repeat(MAX_PASSWORD_LEN + 1))),
            Err(PasswordError::TooLong)
        ));
        // Counted in characters, not bytes.
        assert!(matches!(
            validate(&s("ééééé")),
            Err(PasswordError::TooShort)
        ));
    }

    #[tokio::test]
    async fn unknown_user_verification_is_false() {
        assert!(!verify_blocking(s("whatever password"), None).await.unwrap());
    }
}
