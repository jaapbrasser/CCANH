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
    Organization(#[from] crate::organization::OrgError),
    #[error(transparent)]
    Dashboard(#[from] crate::dashboards::DashboardError),
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
    pub groups: usize,
}

/// The demo group tree (SPEC §9 example): (name, parent).
const GROUPS: [(&str, Option<&str>); 12] = [
    ("Site A", None),
    ("Building 1", Some("Site A")),
    ("Crane 1", Some("Building 1")),
    ("Crane 2", Some("Building 1")),
    ("Building 2", Some("Site A")),
    ("Crane 3", Some("Building 2")),
    ("Crane 4", Some("Building 2")),
    ("Yard", Some("Site A")),
    ("Site B", None),
    ("Loading Area", Some("Site B")),
    ("Storage Area", Some("Site B")),
    ("Perimeter", Some("Site B")),
];

/// Camera name prefix → group.
const GROUP_OF: [(&str, &str); 8] = [
    ("A1 Crane 1", "Crane 1"),
    ("A1 Crane 2", "Crane 2"),
    ("A2 Crane 3", "Crane 3"),
    ("A2 Crane 4", "Crane 4"),
    ("A Yard", "Yard"),
    ("B Loading", "Loading Area"),
    ("B Storage", "Storage Area"),
    ("B Perimeter", "Perimeter"),
];

/// Tag → camera name substrings that get it.
const TAGS: [(&str, &[&str]); 5] = [
    ("Hook", &["Hook"]),
    ("Cabin", &["Cabin"]),
    ("Gate", &["Gate", "Entrance"]),
    ("Outdoor", &["Yard", "Perimeter", "Loading"]),
    ("Critical", &["Hook", "Gate North", "Entrance"]),
];

/// Each demo user's favorites.
const FAVORITES: [(&str, &[&str]); 2] = [
    ("admin", &["A Yard – Gate North", "A Yard – Overview"]),
    (
        "viewer",
        &[
            "A1 Crane 1 – Hook",
            "A2 Crane 3 – Hook",
            "B Loading – Dock 1",
        ],
    ),
];

async fn seed_organization(db: &SqlitePool) -> Result<usize, DemoError> {
    use crate::organization::{favorites, groups, tags, NewGroup};
    use std::collections::HashMap;

    let mut ids: HashMap<&str, uuid::Uuid> = HashMap::new();
    for (name, parent) in GROUPS {
        let g = groups::create(
            db,
            &NewGroup {
                name: name.to_owned(),
                parent_id: parent.and_then(|p| ids.get(p).copied()),
            },
        )
        .await?;
        ids.insert(name, g.id);
    }
    let (cams, _) = cameras::list(
        db,
        cameras::ListParams {
            limit: i64::MAX,
            ..Default::default()
        },
    )
    .await?;
    let mut tag_ids = Vec::new();
    for (tag, _) in TAGS {
        tag_ids.push(tags::create(db, tag).await?.id);
    }
    for cam in &cams {
        if let Some((_, group)) = GROUP_OF.iter().find(|(p, _)| cam.name.starts_with(p)) {
            groups::add_cameras(db, &[cam.id], &[ids[group]]).await?;
        }
        for ((_, matches), tag) in TAGS.iter().zip(&tag_ids) {
            if matches.iter().any(|m| cam.name.contains(m)) {
                tags::add_cameras(db, &[cam.id], &[*tag]).await?;
            }
        }
    }
    for (username, names) in FAVORITES {
        let Some(user) = users::find_credentials(db, username).await? else {
            continue;
        };
        for cam in cams.iter().filter(|c| names.contains(&c.name.as_str())) {
            favorites::add(db, user.user.id, cam.id).await?;
        }
    }
    seed_dashboards(db, &cams).await?;
    Ok(GROUPS.len())
}

/// Two sample dashboards per demo account (SPEC §12.1): (name, rows, cols,
/// camera name prefixes in tile order).
const DASHBOARDS: [(&str, u8, u8, &[&str]); 2] = [
    ("Site A cranes", 3, 3, &["A1 Crane", "A2 Crane"]),
    (
        "Gates and docks",
        2,
        3,
        &["A Yard – Gate", "B Perimeter – Entrance", "B Loading"],
    ),
];

async fn seed_dashboards(
    db: &SqlitePool,
    cams: &[cameras::CameraSummary],
) -> Result<(), DemoError> {
    use crate::dashboards::{self, NewDashboard, Tile};
    for account in ACCOUNTS {
        let Some(user) = users::find_credentials(db, account.username).await? else {
            continue;
        };
        for (name, rows, cols, prefixes) in DASHBOARDS {
            let cells = usize::from(rows) * usize::from(cols);
            let tiles = cams
                .iter()
                .filter(|c| prefixes.iter().any(|p| c.name.starts_with(p)))
                .take(cells)
                .enumerate()
                .map(|(i, c)| Tile {
                    position: u16::try_from(i).unwrap_or(0),
                    camera_id: c.id,
                })
                .collect();
            dashboards::create(
                db,
                user.user.id,
                &NewDashboard {
                    name: name.to_owned(),
                    rows,
                    cols,
                    tiles,
                },
            )
            .await?;
        }
    }
    Ok(())
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
        summary.groups = seed_organization(db).await?;
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
                cameras: FLEET.len(),
                groups: GROUPS.len(),
            }
        );
        assert_eq!(
            seed(&db, &store).await.unwrap(),
            SeedSummary::default(),
            "idempotent"
        );

        // Site A holds the 8 crane cameras (via Building → Crane) and the 4 yard ones.
        let groups = crate::organization::groups::list(&db).await.unwrap();
        let site_a = groups.iter().find(|g| g.name == "Site A").unwrap().id;
        let (_, total) = cameras::list(
            &db,
            cameras::ListParams {
                group: Some(site_a),
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(total, 12);
        let viewer = users::find_credentials(&db, "viewer")
            .await
            .unwrap()
            .unwrap();
        let favs = crate::organization::favorites::list(&db, viewer.user.id)
            .await
            .unwrap();
        assert_eq!(favs.len(), 3);

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
