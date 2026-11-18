//! User accounts: model and repository.
//!
//! The "last enabled admin can't be deleted, disabled or demoted" rule is enforced
//! inside single SQL statements, so concurrent requests can't race past it.

use serde::Serialize;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use crate::auth::Role;

pub const MAX_USERNAME_LEN: usize = 64;

/// A user account as returned by the API. Never includes the password hash.
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
    pub disabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub password_changed_at: OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum UserError {
    #[error("username is already taken")]
    UsernameTaken,
    #[error("user not found")]
    NotFound,
    #[error("this would leave no enabled admin")]
    LastAdmin,
    #[error("invalid username: {0}")]
    InvalidUsername(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Usernames: 1–64 characters, letters, digits and `. _ - @`, trimmed.
pub fn validate_username(username: &str) -> Result<(), UserError> {
    if username.is_empty() {
        return Err(UserError::InvalidUsername("must not be empty"));
    }
    if username.chars().count() > MAX_USERNAME_LEN {
        return Err(UserError::InvalidUsername("must be at most 64 characters"));
    }
    if username != username.trim() {
        return Err(UserError::InvalidUsername(
            "must not start or end with whitespace",
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
    {
        return Err(UserError::InvalidUsername(
            "may only contain letters, digits, '.', '_', '-' and '@'",
        ));
    }
    Ok(())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Row shape for [`User`] queries, with SQLx macro type overrides in each query.
struct UserRow {
    id: Hyphenated,
    username: String,
    role: Role,
    disabled: bool,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    password_changed_at: OffsetDateTime,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        User {
            id: row.id.into_uuid(),
            username: row.username,
            role: row.role,
            disabled: row.disabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
            password_changed_at: row.password_changed_at,
        }
    }
}

pub async fn count(db: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT COUNT(*) AS "n!: i64" FROM users"#)
        .fetch_one(db)
        .await
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    let id = id.hyphenated();
    let row = sqlx::query_as!(
        UserRow,
        r#"SELECT id AS "id: Hyphenated", username, role AS "role: Role",
                  disabled AS "disabled: bool", created_at AS "created_at: OffsetDateTime",
                  updated_at AS "updated_at: OffsetDateTime",
                  password_changed_at AS "password_changed_at: OffsetDateTime"
             FROM users WHERE id = ?1"#,
        id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(User::from))
}

/// A user plus their password hash, for login only.
pub struct Credentials {
    pub user: User,
    pub password_hash: String,
}

pub async fn find_credentials(
    db: &SqlitePool,
    username: &str,
) -> Result<Option<Credentials>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT id AS "id: Hyphenated", username, role AS "role: Role",
            disabled AS "disabled: bool", created_at AS "created_at: OffsetDateTime",
            updated_at AS "updated_at: OffsetDateTime",
            password_changed_at AS "password_changed_at: OffsetDateTime", password_hash
           FROM users WHERE username = ?1"#,
        username
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| Credentials {
        user: User {
            id: r.id.into_uuid(),
            username: r.username,
            role: r.role,
            disabled: r.disabled,
            created_at: r.created_at,
            updated_at: r.updated_at,
            password_changed_at: r.password_changed_at,
        },
        password_hash: r.password_hash,
    }))
}

pub async fn password_hash(db: &SqlitePool, id: Uuid) -> Result<Option<String>, sqlx::Error> {
    let id = id.hyphenated();
    sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = ?1", id)
        .fetch_optional(db)
        .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserSort {
    #[default]
    Username,
    CreatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

pub struct ListParams<'a> {
    pub search: Option<&'a str>,
    pub sort: UserSort,
    pub order: SortOrder,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list(db: &SqlitePool, p: ListParams<'_>) -> Result<(Vec<User>, i64), sqlx::Error> {
    let pattern = p.search.map(|s| format!("%{}%", escape_like(s)));
    let sort = match p.sort {
        UserSort::Username => "username",
        UserSort::CreatedAt => "created_at",
    };
    let order = match p.order {
        SortOrder::Asc => "asc",
        SortOrder::Desc => "desc",
    };

    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM users
           WHERE ?1 IS NULL OR username LIKE ?1 ESCAPE '\'"#,
        pattern
    )
    .fetch_one(db)
    .await?;

    // ORDER BY can't be a bind parameter, so each sort/order pair is a CASE arm.
    let rows = sqlx::query_as!(
        UserRow,
        r#"SELECT id AS "id: Hyphenated", username, role AS "role: Role",
                  disabled AS "disabled: bool", created_at AS "created_at: OffsetDateTime",
                  updated_at AS "updated_at: OffsetDateTime",
                  password_changed_at AS "password_changed_at: OffsetDateTime"
             FROM users
            WHERE ?1 IS NULL OR username LIKE ?1 ESCAPE '\'
            ORDER BY
                 CASE WHEN ?2 = 'username'   AND ?3 = 'asc'  THEN username   END ASC,
                 CASE WHEN ?2 = 'username'   AND ?3 = 'desc' THEN username   END DESC,
                 CASE WHEN ?2 = 'created_at' AND ?3 = 'asc'  THEN created_at END ASC,
                 CASE WHEN ?2 = 'created_at' AND ?3 = 'desc' THEN created_at END DESC,
                 id ASC
            LIMIT ?4 OFFSET ?5"#,
        pattern,
        sort,
        order,
        p.limit,
        p.offset
    )
    .fetch_all(db)
    .await?;

    Ok((rows.into_iter().map(User::from).collect(), total))
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn create(
    db: &SqlitePool,
    username: &str,
    password_hash: &str,
    role: Role,
) -> Result<User, UserError> {
    validate_username(username)?;
    let id = Uuid::now_v7();
    let hyphenated = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let result = sqlx::query!(
        "INSERT INTO users (id, username, password_hash, role, disabled, created_at, updated_at, password_changed_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5, ?5)",
        hyphenated,
        username,
        password_hash,
        role,
        now
    )
    .execute(db)
    .await;
    match result {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(UserError::UsernameTaken),
        Err(e) => return Err(e.into()),
    }
    get(db, id).await?.ok_or(UserError::NotFound)
}

/// Create the first admin, but only if no users exist (atomic, for the setup flow).
/// Returns `Ok(None)` if a user already exists.
pub async fn create_first_admin(
    db: &SqlitePool,
    username: &str,
    password_hash: &str,
) -> Result<Option<User>, UserError> {
    validate_username(username)?;
    let id = Uuid::now_v7();
    let hyphenated = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let role = Role::Admin;
    let result = sqlx::query!(
        "INSERT INTO users (id, username, password_hash, role, disabled, created_at, updated_at, password_changed_at)
         SELECT ?1, ?2, ?3, ?4, 0, ?5, ?5, ?5
         WHERE NOT EXISTS (SELECT 1 FROM users)",
        hyphenated,
        username,
        password_hash,
        role,
        now
    )
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    Ok(get(db, id).await?)
}

/// Change role and/or disabled flag. Refuses to remove the last enabled admin.
pub async fn update(
    db: &SqlitePool,
    id: Uuid,
    role: Option<Role>,
    disabled: Option<bool>,
) -> Result<User, UserError> {
    let hyphenated = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    // The guard: if this row is currently an enabled admin and won't be after the
    // update, another enabled admin must exist. Evaluated atomically by SQLite.
    let result = sqlx::query!(
        "UPDATE users
            SET role = COALESCE(?2, role),
                disabled = COALESCE(?3, disabled),
                updated_at = ?4
          WHERE id = ?1
            AND (
              NOT (role = 'admin' AND disabled = 0)
              OR (COALESCE(?2, role) = 'admin' AND COALESCE(?3, disabled) = 0)
              OR EXISTS (SELECT 1 FROM users o
                          WHERE o.id != ?1 AND o.role = 'admin' AND o.disabled = 0)
            )",
        hyphenated,
        role,
        disabled,
        now
    )
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return match get(db, id).await? {
            None => Err(UserError::NotFound),
            Some(_) => Err(UserError::LastAdmin),
        };
    }
    get(db, id).await?.ok_or(UserError::NotFound)
}

/// Delete a user. Refuses to delete the last enabled admin. Sessions cascade.
pub async fn delete(db: &SqlitePool, id: Uuid) -> Result<(), UserError> {
    let hyphenated = id.hyphenated();
    let result = sqlx::query!(
        "DELETE FROM users
          WHERE id = ?1
            AND (
              NOT (role = 'admin' AND disabled = 0)
              OR EXISTS (SELECT 1 FROM users o
                          WHERE o.id != ?1 AND o.role = 'admin' AND o.disabled = 0)
            )",
        hyphenated
    )
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return match get(db, id).await? {
            None => Err(UserError::NotFound),
            Some(_) => Err(UserError::LastAdmin),
        };
    }
    Ok(())
}

pub async fn set_password(db: &SqlitePool, id: Uuid, password_hash: &str) -> Result<(), UserError> {
    let hyphenated = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let result = sqlx::query!(
        "UPDATE users SET password_hash = ?2, password_changed_at = ?3, updated_at = ?3 WHERE id = ?1",
        hyphenated,
        password_hash,
        now
    )
    .execute(db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(UserError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    async fn db() -> SqlitePool {
        storage::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn create_and_fetch() {
        let db = db().await;
        let user = create(&db, "Alice", "hash", Role::Admin).await.unwrap();
        assert_eq!(user.username, "Alice");
        assert_eq!(user.role, Role::Admin);
        assert!(!user.disabled);
        assert_eq!(get(&db, user.id).await.unwrap(), Some(user.clone()));
        let creds = find_credentials(&db, "alice").await.unwrap().unwrap();
        assert_eq!(creds.user.id, user.id, "lookup is case-insensitive");
        assert_eq!(creds.password_hash, "hash");
    }

    #[tokio::test]
    async fn usernames_are_unique_case_insensitively() {
        let db = db().await;
        create(&db, "alice", "h", Role::Viewer).await.unwrap();
        assert!(matches!(
            create(&db, "ALICE", "h", Role::Viewer).await,
            Err(UserError::UsernameTaken)
        ));
    }

    #[tokio::test]
    async fn username_validation() {
        assert!(validate_username("alice.smith@site-a_1").is_ok());
        assert!(validate_username("").is_err());
        assert!(validate_username(" alice").is_err());
        assert!(validate_username("alice smith").is_err());
        assert!(validate_username("<script>").is_err());
        assert!(validate_username(&"a".repeat(65)).is_err());
    }

    #[tokio::test]
    async fn last_enabled_admin_is_protected() {
        let db = db().await;
        let admin = create(&db, "admin", "h", Role::Admin).await.unwrap();
        let viewer = create(&db, "viewer", "h", Role::Viewer).await.unwrap();

        assert!(matches!(
            update(&db, admin.id, Some(Role::Viewer), None).await,
            Err(UserError::LastAdmin)
        ));
        assert!(matches!(
            update(&db, admin.id, None, Some(true)).await,
            Err(UserError::LastAdmin)
        ));
        assert!(matches!(
            delete(&db, admin.id).await,
            Err(UserError::LastAdmin)
        ));
        // No-op updates on the last admin are fine.
        update(&db, admin.id, Some(Role::Admin), Some(false))
            .await
            .unwrap();

        // With a second enabled admin, the first can be demoted.
        update(&db, viewer.id, Some(Role::Admin), None)
            .await
            .unwrap();
        let demoted = update(&db, admin.id, Some(Role::Viewer), None)
            .await
            .unwrap();
        assert_eq!(demoted.role, Role::Viewer);

        // A disabled admin doesn't count as the remaining admin.
        let third = create(&db, "third", "h", Role::Admin).await.unwrap();
        update(&db, third.id, None, Some(true)).await.unwrap();
        assert!(matches!(
            delete(&db, viewer.id).await,
            Err(UserError::LastAdmin)
        ));
    }

    #[tokio::test]
    async fn update_and_delete_missing_user() {
        let db = db().await;
        let id = Uuid::now_v7();
        assert!(matches!(
            update(&db, id, Some(Role::Viewer), None).await,
            Err(UserError::NotFound)
        ));
        assert!(matches!(delete(&db, id).await, Err(UserError::NotFound)));
        assert!(matches!(
            set_password(&db, id, "h").await,
            Err(UserError::NotFound)
        ));
    }

    #[tokio::test]
    async fn first_admin_only_when_empty() {
        let db = db().await;
        let first = create_first_admin(&db, "root", "h").await.unwrap();
        assert!(first.is_some());
        assert!(create_first_admin(&db, "second", "h")
            .await
            .unwrap()
            .is_none());
        assert_eq!(count(&db).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn list_searches_sorts_and_paginates() {
        let db = db().await;
        for name in ["carol", "alice", "bob", "al_x"] {
            create(&db, name, "h", Role::Viewer).await.unwrap();
        }
        let params = |search, sort, order, limit, offset| ListParams {
            search,
            sort,
            order,
            limit,
            offset,
        };

        let (users, total) = list(&db, params(None, UserSort::Username, SortOrder::Asc, 2, 0))
            .await
            .unwrap();
        assert_eq!(total, 4);
        let names: Vec<_> = users.iter().map(|u| u.username.as_str()).collect();
        assert_eq!(names, ["al_x", "alice"]);

        let (users, _) = list(
            &db,
            params(None, UserSort::Username, SortOrder::Desc, 10, 0),
        )
        .await
        .unwrap();
        assert_eq!(users[0].username, "carol");

        let (users, _) = list(
            &db,
            params(None, UserSort::CreatedAt, SortOrder::Asc, 10, 0),
        )
        .await
        .unwrap();
        assert_eq!(users[0].username, "carol");

        // `_` is matched literally, not as a LIKE wildcard.
        let (users, total) = list(
            &db,
            params(Some("l_"), UserSort::Username, SortOrder::Asc, 10, 0),
        )
        .await
        .unwrap();
        assert_eq!(total, 1);
        assert_eq!(users[0].username, "al_x");
    }
}
