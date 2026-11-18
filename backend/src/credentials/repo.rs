//! Credential set persistence. Only ciphertext is stored or returned from here;
//! callers decrypt with a [`CredentialStore`] at the point of use.

use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use super::{CredentialError, CredentialStore, Credentials};

pub const MAX_SET_NAME_LEN: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum CredentialSetError {
    #[error("credential set not found")]
    NotFound,
    #[error("a credential set with this name already exists")]
    DuplicateName,
    #[error("credential set is used by {0} camera(s)")]
    InUse(i64),
    #[error("name must be 1–100 characters")]
    InvalidName,
    #[error("username must be 1–128 characters")]
    InvalidUsername,
    #[error(transparent)]
    Crypto(#[from] CredentialError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// A shared credential set as shown to admins. Never contains secrets.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct CredentialSetSummary {
    pub id: Uuid,
    pub name: String,
    /// Number of cameras using this set.
    pub camera_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

pub fn validate_name(name: &str) -> Result<String, CredentialSetError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > MAX_SET_NAME_LEN {
        return Err(CredentialSetError::InvalidName);
    }
    Ok(n.to_owned())
}

pub fn validate_username(username: &str) -> Result<(), CredentialSetError> {
    if username.is_empty() || username.chars().count() > 128 {
        return Err(CredentialSetError::InvalidUsername);
    }
    Ok(())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

pub async fn list_shared(db: &SqlitePool) -> Result<Vec<CredentialSetSummary>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT s.id AS "id: Hyphenated", s.name AS "name!",
                  (SELECT COUNT(*) FROM cameras c WHERE c.credential_set_id = s.id) AS "camera_count!: i64",
                  s.created_at AS "created_at: OffsetDateTime",
                  s.updated_at AS "updated_at: OffsetDateTime"
             FROM credential_sets s WHERE s.shared = 1
            ORDER BY s.name COLLATE NOCASE"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| CredentialSetSummary {
            id: r.id.into_uuid(),
            name: r.name,
            camera_count: r.camera_count,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn get_shared(
    db: &SqlitePool,
    id: Uuid,
) -> Result<Option<CredentialSetSummary>, sqlx::Error> {
    Ok(list_shared(db).await?.into_iter().find(|s| s.id == id))
}

pub async fn create_shared(
    db: &SqlitePool,
    store: &dyn CredentialStore,
    name: &str,
    creds: &Credentials,
) -> Result<CredentialSetSummary, CredentialSetError> {
    let name = validate_name(name)?;
    validate_username(&creds.username)?;
    let secret = store.seal(creds)?;
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let r = sqlx::query!(
        "INSERT INTO credential_sets (id, name, shared, secret, created_at, updated_at)
         VALUES (?1, ?2, 1, ?3, ?4, ?4)",
        hid,
        name,
        secret,
        now
    )
    .execute(db)
    .await;
    match r {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(CredentialSetError::DuplicateName),
        Err(e) => return Err(e.into()),
    }
    get_shared(db, id)
        .await?
        .ok_or(CredentialSetError::NotFound)
}

/// Rename a shared set and/or replace its credentials.
pub async fn update_shared(
    db: &SqlitePool,
    store: &dyn CredentialStore,
    id: Uuid,
    name: Option<&str>,
    creds: Option<&Credentials>,
) -> Result<CredentialSetSummary, CredentialSetError> {
    let hid = id.hyphenated();
    let name = name.map(validate_name).transpose()?;
    if let Some(c) = creds {
        validate_username(&c.username)?;
    }
    let secret = creds.map(|c| store.seal(c)).transpose()?;
    let now = OffsetDateTime::now_utc();
    let r = sqlx::query!(
        "UPDATE credential_sets
            SET name = COALESCE(?2, name), secret = COALESCE(?3, secret), updated_at = ?4
          WHERE id = ?1 AND shared = 1",
        hid,
        name,
        secret,
        now
    )
    .execute(db)
    .await;
    match r {
        Ok(r) if r.rows_affected() == 0 => return Err(CredentialSetError::NotFound),
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(CredentialSetError::DuplicateName),
        Err(e) => return Err(e.into()),
    }
    get_shared(db, id)
        .await?
        .ok_or(CredentialSetError::NotFound)
}

pub async fn delete_shared(db: &SqlitePool, id: Uuid) -> Result<(), CredentialSetError> {
    let hid = id.hyphenated();
    let in_use = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM cameras WHERE credential_set_id = ?1"#,
        hid
    )
    .fetch_one(db)
    .await?;
    if in_use > 0 {
        return Err(CredentialSetError::InUse(in_use));
    }
    let r = sqlx::query!(
        "DELETE FROM credential_sets WHERE id = ?1 AND shared = 1",
        hid
    )
    .execute(db)
    .await;
    match r {
        Ok(r) if r.rows_affected() == 0 => Err(CredentialSetError::NotFound),
        Ok(_) => Ok(()),
        // A camera was attached between the count and the delete.
        Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => {
            Err(CredentialSetError::InUse(1))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn is_shared(db: &SqlitePool, id: Uuid) -> Result<bool, sqlx::Error> {
    let hid = id.hyphenated();
    Ok(sqlx::query_scalar!(
        r#"SELECT shared AS "shared: bool" FROM credential_sets WHERE id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?
    .unwrap_or(false))
}

/// Store credentials entered for a single camera. Returns the new set's id.
pub async fn create_private(
    conn: &mut SqliteConnection,
    store: &dyn CredentialStore,
    creds: &Credentials,
) -> Result<Uuid, CredentialSetError> {
    validate_username(&creds.username)?;
    let secret = store.seal(creds)?;
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    sqlx::query!(
        "INSERT INTO credential_sets (id, name, shared, secret, created_at, updated_at)
         VALUES (?1, NULL, 0, ?2, ?3, ?3)",
        hid,
        secret,
        now
    )
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

/// Delete a private set if nothing references it any more.
pub async fn delete_private_if_unused(
    conn: &mut SqliteConnection,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    sqlx::query!(
        "DELETE FROM credential_sets
          WHERE id = ?1 AND shared = 0
            AND NOT EXISTS (SELECT 1 FROM cameras WHERE credential_set_id = ?1)",
        hid
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn secret(db: &SqlitePool, id: Uuid) -> Result<Option<Vec<u8>>, sqlx::Error> {
    let hid = id.hyphenated();
    sqlx::query_scalar!("SELECT secret FROM credential_sets WHERE id = ?1", hid)
        .fetch_optional(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::AesGcmStore;
    use crate::secret::{ExposeSecret, SecretString};

    fn creds(user: &str) -> Credentials {
        Credentials {
            username: user.into(),
            password: SecretString::from("camera-secret-123".to_owned()),
        }
    }

    #[tokio::test]
    async fn shared_set_lifecycle() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let store: Box<dyn CredentialStore> = Box::new(AesGcmStore::ephemeral());
        let set = create_shared(&db, &*store, "Site A", &creds("root"))
            .await
            .unwrap();
        assert_eq!(set.camera_count, 0);
        assert!(matches!(
            create_shared(&db, &*store, "site a", &creds("x")).await,
            Err(CredentialSetError::DuplicateName)
        ));

        let raw = secret(&db, set.id).await.unwrap().unwrap();
        assert!(!raw.windows(17).any(|w| w == b"camera-secret-123"));
        assert_eq!(store.open(&raw).unwrap().username, "root");

        let renamed = update_shared(
            &db,
            &*store,
            set.id,
            Some("Site A cams"),
            Some(&creds("admin")),
        )
        .await
        .unwrap();
        assert_eq!(renamed.name, "Site A cams");
        let opened = store
            .open(&secret(&db, set.id).await.unwrap().unwrap())
            .unwrap();
        assert_eq!(opened.username, "admin");
        assert_eq!(opened.password.expose_secret(), "camera-secret-123");

        delete_shared(&db, set.id).await.unwrap();
        assert!(list_shared(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn private_sets_are_hidden_and_cleaned_up() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let store: Box<dyn CredentialStore> = Box::new(AesGcmStore::ephemeral());
        // The in-memory test pool has one connection: release it between steps.
        let id = {
            let mut conn = db.acquire().await.unwrap();
            create_private(&mut conn, &*store, &creds("root"))
                .await
                .unwrap()
        };
        assert!(list_shared(&db).await.unwrap().is_empty());
        assert!(!is_shared(&db, id).await.unwrap());
        {
            let mut conn = db.acquire().await.unwrap();
            delete_private_if_unused(&mut conn, id).await.unwrap();
        }
        assert!(secret(&db, id).await.unwrap().is_none());
    }
}
