//! Authentication and authorization: roles, password hashing, sessions, login
//! rate limiting, request extractors and the CSRF guard.
//!
//! Role checks live in exactly one place: the [`AuthUser`] and [`AdminUser`]
//! extractors. Handlers declare the role they need by taking one of them as their
//! first argument. See ADR 0004 for the session and CSRF design.

pub mod csrf;
pub mod extract;
pub mod password;
pub mod rate_limit;
pub mod session;
pub mod setup;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

pub use extract::{AdminUser, AuthUser, ClientIp};

/// User role. Authorization is written so per-group camera access can be layered
/// on later (SPEC §14.1, §18 Q4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Viewer => "viewer",
        }
    }
}

/// Number of random bytes in session and setup tokens (256 bits).
pub const TOKEN_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("operating system random number generator failed: {0}")]
pub struct RandomError(String);

/// Generate a URL-safe random token.
pub fn new_token() -> Result<String, RandomError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| RandomError(e.to_string()))?;
    Ok(encode_token(&bytes))
}

pub fn encode_token(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// SHA-256 of a token, hex-encoded. Only this is stored server-side.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_url_safe() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43); // 32 bytes, base64url, no padding
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn token_hash_is_stable_hex() {
        let h = hash_token("abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
