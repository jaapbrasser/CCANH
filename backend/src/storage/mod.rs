//! SQLite connection pool and migrations.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("failed to create data directory {path}: {source}")]
    DataDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to back up database before migrating: {0}")]
    Backup(std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Open (creating if needed) the SQLite database at `path` in WAL mode, back it up
/// if there are pending migrations, and run them.
pub async fn connect(path: &Path) -> Result<SqlitePool, StorageError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|source| StorageError::DataDir {
            path: dir.to_path_buf(),
            source,
        })?;
    }

    let existed = path.is_file();
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?;

    if existed && has_pending_migrations(&pool).await? {
        backup(&pool, path).await?;
    }

    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// In-memory database for tests. A single connection keeps the memory DB alive.
pub async fn connect_in_memory() -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

async fn has_pending_migrations(pool: &SqlitePool) -> Result<bool, StorageError> {
    let table_exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?;
    if table_exists.is_none() {
        // An existing file without migration history: treat as pending so it gets
        // backed up before we touch it.
        return Ok(true);
    }
    let applied: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
            .fetch_all(pool)
            .await?;
    Ok(MIGRATOR
        .iter()
        .any(|m| !m.migration_type.is_down_migration() && !applied.contains(&m.version)))
}

/// Write a consistent snapshot next to the database using `VACUUM INTO`, which is
/// safe while WAL is active.
async fn backup(pool: &SqlitePool, path: &Path) -> Result<(), StorageError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".bak-{stamp}"));
    let target = path.with_file_name(name);
    if target.exists() {
        std::fs::remove_file(&target).map_err(StorageError::Backup)?;
    }
    sqlx::query("VACUUM INTO ?")
        .bind(target.to_string_lossy().into_owned())
        .execute(pool)
        .await?;
    tracing::info!(backup = %target.display(), "backed up database before applying migrations");
    Ok(())
}

/// Lightweight liveness check used by the health endpoint.
pub async fn ping(pool: &SqlitePool) -> Result<(), StorageError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_database_in_wal_mode_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("test.db");
        let pool = connect(&path).await.unwrap();

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fk, 1);

        let version: String = sqlx::query_scalar("SELECT value FROM app_meta WHERE key = 'schema'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, "1");
        ping(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn backs_up_existing_database_without_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        {
            let options = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .connect_with(options)
                .await
                .unwrap();
            sqlx::query("CREATE TABLE legacy (id INTEGER)")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        }

        let pool = connect(&path).await.unwrap();
        pool.close().await;

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .collect();
        assert_eq!(backups.len(), 1);
    }

    #[tokio::test]
    async fn reopening_up_to_date_database_makes_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.db");
        connect(&path).await.unwrap().close().await;
        connect(&path).await.unwrap().close().await;
        let backups = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .count();
        assert_eq!(backups, 0);
    }
}
