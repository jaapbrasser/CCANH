//! First-run bootstrap (SPEC §14.2).
//!
//! When the database has no users, the server generates a one-time setup token at
//! startup and prints it to the console and the log. Presenting it to
//! `POST /api/v1/setup` creates the first admin. Only the token's hash is kept,
//! in memory; it is cleared once used and never valid once any user exists.

use std::sync::Mutex;

use sqlx::SqlitePool;

use super::{hash_token, new_token, password, RandomError, Role};
use crate::audit::{self, action, Event, Outcome};
use crate::secret::SecretString;
use crate::users::{self, User, UserError};

#[derive(Debug, thiserror::Error)]
pub enum CreateAdminError {
    #[error(transparent)]
    Password(#[from] password::PasswordError),
    #[error(transparent)]
    User(#[from] UserError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Create an admin account from the server console (`crane --create-admin`).
/// Console access implies control of the host, so this works whether or not
/// users already exist — it doubles as the recovery path for a lost admin password.
pub async fn create_admin(
    db: &SqlitePool,
    username: &str,
    password: SecretString,
) -> Result<User, CreateAdminError> {
    users::validate_username(username)?;
    password::validate(&password)?;
    let hash = password::hash_blocking(password).await?;
    let user = users::create(db, username, &hash, Role::Admin).await?;
    audit::record(
        db,
        Event::new(action::USER_CREATE, Outcome::Success)
            .actor(None, "console")
            .target("user", user.id)
            .details(serde_json::json!({ "username": user.username, "role": user.role, "via": "--create-admin" })),
    )
    .await?;
    Ok(user)
}

#[derive(Debug, Default)]
pub struct SetupState {
    token_hash: Mutex<Option<String>>,
}

impl SetupState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.token_hash.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Generate a fresh token, replacing any previous one, and return it.
    pub fn issue(&self) -> Result<String, RandomError> {
        let token = new_token()?;
        *self.lock() = Some(hash_token(&token));
        Ok(token)
    }

    pub fn is_pending(&self) -> bool {
        self.lock().is_some()
    }

    /// Check a presented token without consuming it.
    pub fn verify(&self, token: &str) -> bool {
        // Comparing SHA-256 digests: timing differences reveal nothing useful
        // about the token itself.
        self.lock().as_deref() == Some(hash_token(token).as_str())
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_admin_validates_and_audits() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let weak = create_admin(&db, "root", SecretString::from("short".to_owned())).await;
        assert!(matches!(weak, Err(CreateAdminError::Password(_))));

        let user = create_admin(
            &db,
            "root",
            SecretString::from("a long enough pw".to_owned()),
        )
        .await
        .unwrap();
        assert_eq!(user.role, Role::Admin);
        let (entries, _) = audit::list(&db, audit::Filter::default(), 10, 0)
            .await
            .unwrap();
        assert_eq!(entries[0].actor_username.as_deref(), Some("console"));

        let dup = create_admin(
            &db,
            "ROOT",
            SecretString::from("a long enough pw".to_owned()),
        )
        .await;
        assert!(matches!(
            dup,
            Err(CreateAdminError::User(UserError::UsernameTaken))
        ));
    }

    #[test]
    fn issue_verify_clear() {
        let state = SetupState::default();
        assert!(!state.is_pending());
        assert!(!state.verify(""));
        let token = state.issue().unwrap();
        assert!(state.is_pending());
        assert!(state.verify(&token));
        assert!(!state.verify("nope"));
        state.clear();
        assert!(!state.verify(&token));
    }
}
