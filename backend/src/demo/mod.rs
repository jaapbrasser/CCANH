//! Demo mode (SPEC §12): a separate `demo.db`, freshly seeded on each start
//! unless `--demo-keep-state`, with demo users and the simulated camera fleet.
//!
//! Nothing here runs in production mode: `main` only calls it when
//! `config.mode == Mode::Demo`, and demo data lives in its own database file.

use std::path::Path;

use serde::Serialize;
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::auth::{password, Role};
use crate::cameras::{self, CameraSettings, Scheme, TlsMode};
use crate::credentials::CredentialStore;
use crate::providers::demo::catalog::FLEET;
use crate::secret::SecretString;
use crate::users;

/// A documented demo account. These exist only in `demo.db`.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct DemoAccount {
    pub username: &'static str,
    pub password: &'static str,
    pub role: Role,
}

pub const ACCOUNTS: [DemoAccount; 2] = [
    DemoAccount {
        username: "admin",
        password: "crane-demo-admin",
        role: Role::Admin,
    },
    DemoAccount {
        username: "viewer",
        password: "crane-demo-viewer",
        role: Role::Viewer,
    },
];

#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("failed to remove old demo database {path}: {source}")]
    Reset {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Password(#[from] password::PasswordError),
    #[error(transparent)]
    User(#[from] users::UserError),
    #[error(transparent)]
    Camera(#[from] cameras::CameraError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Delete the demo database and its WAL/SHM files so the next connect starts
/// fresh. Only ever called with the demo database path.
pub fn reset_database_file(path: &Path) -> Result<(), DemoError> {
    debug_assert!(
        path.file_name().is_some_and(|n| n == "demo.db"),
        "refusing to reset a non-demo database"
    );
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let file = Path::new(&name);
        if file.exists() {
            std::fs::remove_file(file).map_err(|source| DemoError::Reset {
                path: file.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedSummary {
    pub users: usize,
    pub cameras: usize,
}

/// Seed demo users and cameras into an empty database. Does nothing to tables
/// that already have rows, so `--demo-keep-state` keeps edits.
pub async fn seed(db: &SqlitePool, store: &dyn CredentialStore) -> Result<SeedSummary, DemoError> {
    let mut summary = SeedSummary::default();
    if users::count(db).await? == 0 {
        for account in ACCOUNTS {
            let hash =
                password::hash_blocking(SecretString::from(account.password.to_owned())).await?;
            users::create(db, account.username, &hash, account.role).await?;
            summary.users += 1;
        }
    }
    if cameras::count(db).await? == 0 {
        for cam in FLEET {
            let settings = CameraSettings {
                name: cam.name.to_owned(),
                host: cam.host.to_owned(),
                port: 443,
                scheme: Scheme::Https,
                tls_mode: TlsMode::Strict,
                tls_pinned_sha256: None,
                monitoring_enabled: true,
                allow_basic_over_http: false,
            };
            cameras::create(db, store, &settings, None).await?;
            summary.cameras += 1;
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn seeds_once_with_valid_demo_passwords() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let store = crate::credentials::AesGcmStore::ephemeral();
        let first = seed(&db, &store).await.unwrap();
        assert_eq!(
            first,
            SeedSummary {
                users: 2,
                cameras: FLEET.len()
            }
        );
        assert_eq!(
            seed(&db, &store).await.unwrap(),
            SeedSummary::default(),
            "idempotent"
        );

        for account in ACCOUNTS {
            password::validate(&SecretString::from(account.password.to_owned())).unwrap();
            let creds = users::find_credentials(&db, account.username)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(creds.user.role, account.role);
            assert!(password::verify(
                &SecretString::from(account.password.to_owned()),
                &creds.password_hash
            ));
        }
    }

    #[test]
    fn reset_removes_db_and_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("demo.db");
        for f in ["demo.db", "demo.db-wal", "demo.db-shm", "crane.db"] {
            std::fs::write(dir.path().join(f), b"x").unwrap();
        }
        reset_database_file(&db).unwrap();
        assert!(!db.exists());
        assert!(!dir.path().join("demo.db-wal").exists());
        assert!(
            dir.path().join("crane.db").exists(),
            "production DB untouched"
        );
        reset_database_file(&db).unwrap(); // missing files are fine
    }
}
