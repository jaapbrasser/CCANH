//! Background task manager (SPEC §10.3).
//!
//! Every long-running operation (connection tests now; discovery and bulk
//! operations later) is a tracked task: persisted in SQLite, run with bounded
//! concurrency, cancellable, with progress and a categorized error. Every state
//! change is published on the event bus (WebSocket topic `tasks`).
//!
//! Recurring polling is not a task; it's the supervised status monitor.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use crate::events::{Event, EventBus};
use crate::providers::{ErrorCategory, ProviderError};

/// Tasks running at the same time; others wait as `queued`.
pub const MAX_RUNNING: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Task {
    pub id: Uuid,
    /// What the task does, e.g. `test_connection`.
    pub kind: String,
    pub camera_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_by_username: Option<String>,
    pub status: TaskStatus,
    pub progress_current: u32,
    /// `null` when progress is indeterminate.
    pub progress_total: Option<u32>,
    /// Current step, human readable.
    pub message: Option<String>,
    pub error_category: Option<ErrorCategory>,
    pub error_message: Option<String>,
    /// Kind-specific result, present once the task succeeded (or failed with
    /// partial results).
    #[schema(value_type = Option<Object>)]
    pub result: Option<Value>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub completed_at: Option<OffsetDateTime>,
}

/// Why a task failed. Messages must not contain secrets.
#[derive(Debug, Clone)]
pub struct TaskFailure {
    pub category: ErrorCategory,
    pub message: String,
    /// Optional partial result to keep.
    pub result: Option<Value>,
}

impl From<ProviderError> for TaskFailure {
    fn from(e: ProviderError) -> Self {
        Self {
            category: e.category(),
            message: e.to_string(),
            result: None,
        }
    }
}

impl TaskFailure {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            category: ErrorCategory::Internal,
            message: message.into(),
            result: None,
        }
    }
}

pub type TaskResult = Result<Value, TaskFailure>;

/// Who started a task.
#[derive(Debug, Clone)]
pub struct Creator {
    pub id: Uuid,
    pub username: String,
}

struct Inner {
    db: SqlitePool,
    bus: EventBus,
    running: Mutex<HashMap<Uuid, CancellationToken>>,
    permits: Arc<Semaphore>,
    root: CancellationToken,
}

/// Handle given to a running task.
pub struct TaskContext {
    pub id: Uuid,
    /// Cancelled when the task is cancelled or the server shuts down. Long
    /// steps should `select!` on it.
    pub cancel: CancellationToken,
    inner: Arc<Inner>,
}

impl TaskContext {
    /// Report progress. Errors writing progress are logged, never fatal.
    pub async fn progress(&self, current: u32, total: Option<u32>, message: &str) {
        let id = self.id.hyphenated();
        let cur = i64::from(current);
        let tot = total.map(i64::from);
        let r = sqlx::query!(
            "UPDATE tasks SET progress_current = ?2, progress_total = ?3, message = ?4 WHERE id = ?1",
            id,
            cur,
            tot,
            message
        )
        .execute(&self.inner.db)
        .await;
        if let Err(e) = r {
            tracing::warn!(task_id = %self.id, error = %e, "failed to store task progress");
            return;
        }
        publish(&self.inner, self.id).await;
    }
}

async fn publish(inner: &Inner, id: Uuid) {
    if let Ok(Some(task)) = get(&inner.db, id).await {
        inner.bus.publish(Event::TaskUpdated { task });
    }
}

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<Inner>,
}

#[derive(Debug, thiserror::Error)]
pub enum CancelError {
    #[error("task not found")]
    NotFound,
    #[error("task already finished")]
    AlreadyFinished,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl TaskManager {
    pub fn new(db: SqlitePool, bus: EventBus) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                bus,
                running: Mutex::new(HashMap::new()),
                permits: Arc::new(Semaphore::new(MAX_RUNNING)),
                root: CancellationToken::new(),
            }),
        }
    }

    /// Mark tasks left `queued`/`running` by a previous process as failed
    /// (interrupted). Call once at startup, before accepting requests.
    pub async fn mark_interrupted(&self) -> Result<u64, sqlx::Error> {
        let now = OffsetDateTime::now_utc();
        let r = sqlx::query!(
            "UPDATE tasks SET status = 'failed', error_category = 'internal',
                    error_message = 'interrupted: the server stopped while this task was running',
                    completed_at = ?1
              WHERE status IN ('queued', 'running')",
            now
        )
        .execute(&self.inner.db)
        .await?;
        Ok(r.rows_affected())
    }

    /// Cancel every running task (server shutdown).
    pub fn shutdown(&self) {
        self.inner.root.cancel();
    }

    /// Create a task and run `job` in the background. Returns the queued task.
    pub async fn spawn<F, Fut>(
        &self,
        kind: &'static str,
        camera_id: Option<Uuid>,
        creator: Option<Creator>,
        job: F,
    ) -> Result<Task, sqlx::Error>
    where
        F: FnOnce(TaskContext) -> Fut + Send + 'static,
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        let id = Uuid::now_v7();
        let hid = id.hyphenated();
        let cam = camera_id.map(|c| c.hyphenated());
        let by = creator.as_ref().map(|c| c.id.hyphenated());
        let by_name = creator.as_ref().map(|c| c.username.clone());
        let now = OffsetDateTime::now_utc();
        sqlx::query!(
            "INSERT INTO tasks (id, kind, camera_id, created_by, created_by_username, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6)",
            hid,
            kind,
            cam,
            by,
            by_name,
            now
        )
        .execute(&self.inner.db)
        .await?;

        let cancel = self.inner.root.child_token();
        self.inner
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, cancel.clone());
        let task = get(&self.inner.db, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        self.inner
            .bus
            .publish(Event::TaskUpdated { task: task.clone() });

        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let outcome = run(&inner, id, kind, cancel, job).await;
            inner
                .running
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            if let Err(e) = finish(&inner, id, outcome).await {
                tracing::error!(task_id = %id, error = %e, "failed to store task result");
            }
            publish(&inner, id).await;
        });
        Ok(task)
    }

    /// Request cancellation. The task records `cancelled` when it stops.
    pub async fn cancel(&self, id: Uuid) -> Result<(), CancelError> {
        let token = self
            .inner
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        match token {
            Some(t) => {
                t.cancel();
                Ok(())
            }
            None => match get(&self.inner.db, id).await? {
                None => Err(CancelError::NotFound),
                Some(_) => Err(CancelError::AlreadyFinished),
            },
        }
    }

    pub fn db(&self) -> &SqlitePool {
        &self.inner.db
    }
}

enum Outcome {
    Done(TaskResult),
    Cancelled,
}

async fn run<F, Fut>(
    inner: &Arc<Inner>,
    id: Uuid,
    kind: &str,
    cancel: CancellationToken,
    job: F,
) -> Outcome
where
    F: FnOnce(TaskContext) -> Fut + Send + 'static,
    Fut: Future<Output = TaskResult> + Send + 'static,
{
    // Wait for a slot; cancellation while queued is honoured.
    let permit = tokio::select! {
        () = cancel.cancelled() => return Outcome::Cancelled,
        p = Arc::clone(&inner.permits).acquire_owned() => match p {
            Ok(p) => p,
            Err(_) => return Outcome::Done(Err(TaskFailure::internal("task manager closed"))),
        },
    };
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    if let Err(e) = sqlx::query!(
        "UPDATE tasks SET status = 'running', started_at = ?2 WHERE id = ?1",
        hid,
        now
    )
    .execute(&inner.db)
    .await
    {
        return Outcome::Done(Err(TaskFailure::internal(format!("database: {e}"))));
    }
    publish(inner, id).await;
    tracing::info!(task_id = %id, kind, "task started");

    let ctx = TaskContext {
        id,
        cancel: cancel.clone(),
        inner: Arc::clone(inner),
    };
    let handle = tokio::spawn(job(ctx));
    let abort = handle.abort_handle();
    let outcome = tokio::select! {
        () = cancel.cancelled() => {
            abort.abort();
            Outcome::Cancelled
        }
        joined = handle => match joined {
            Ok(result) => Outcome::Done(result),
            Err(e) if e.is_panic() => Outcome::Done(Err(TaskFailure::internal("the task crashed"))),
            Err(_) => Outcome::Cancelled,
        },
    };
    drop(permit);
    outcome
}

async fn finish(inner: &Inner, id: Uuid, outcome: Outcome) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let (status, category, message, result) = match outcome {
        Outcome::Done(Ok(v)) => (TaskStatus::Succeeded, None, None, Some(v)),
        Outcome::Done(Err(f)) => (
            TaskStatus::Failed,
            Some(f.category),
            Some(f.message),
            f.result,
        ),
        Outcome::Cancelled => (
            TaskStatus::Cancelled,
            None,
            Some(if inner.root.is_cancelled() {
                "cancelled: the server is shutting down".to_owned()
            } else {
                "cancelled".to_owned()
            }),
            None,
        ),
    };
    let result = result.map(|v| v.to_string());
    tracing::info!(task_id = %id, status = ?status, "task finished");
    sqlx::query!(
        "UPDATE tasks SET status = ?2, error_category = ?3, error_message = ?4, result = ?5,
                completed_at = ?6
          WHERE id = ?1",
        hid,
        status,
        category,
        message,
        result,
        now
    )
    .execute(&inner.db)
    .await?;
    Ok(())
}

struct TaskRow {
    id: Hyphenated,
    kind: String,
    camera_id: Option<Hyphenated>,
    created_by: Option<Hyphenated>,
    created_by_username: Option<String>,
    status: TaskStatus,
    progress_current: i64,
    progress_total: Option<i64>,
    message: Option<String>,
    error_category: Option<ErrorCategory>,
    error_message: Option<String>,
    result: Option<String>,
    created_at: OffsetDateTime,
    started_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
}

impl From<TaskRow> for Task {
    fn from(r: TaskRow) -> Self {
        Task {
            id: r.id.into_uuid(),
            kind: r.kind,
            camera_id: r.camera_id.map(Hyphenated::into_uuid),
            created_by: r.created_by.map(Hyphenated::into_uuid),
            created_by_username: r.created_by_username,
            status: r.status,
            progress_current: u32::try_from(r.progress_current).unwrap_or(0),
            progress_total: r.progress_total.and_then(|t| u32::try_from(t).ok()),
            message: r.message,
            error_category: r.error_category,
            error_message: r.error_message,
            result: r.result.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: r.created_at,
            started_at: r.started_at,
            completed_at: r.completed_at,
        }
    }
}

/// Whether a task of `kind` for `camera` is queued or running.
pub async fn active_for(db: &SqlitePool, kind: &str, camera: Uuid) -> Result<bool, sqlx::Error> {
    let cam = camera.hyphenated();
    let n = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM tasks
            WHERE kind = ?1 AND camera_id = ?2 AND status IN ('queued', 'running')"#,
        kind,
        cam
    )
    .fetch_one(db)
    .await?;
    Ok(n > 0)
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<Task>, sqlx::Error> {
    let hid = id.hyphenated();
    let row = sqlx::query_as!(
        TaskRow,
        r#"SELECT id AS "id: Hyphenated", kind, camera_id AS "camera_id: Hyphenated",
                  created_by AS "created_by: Hyphenated", created_by_username,
                  status AS "status: TaskStatus", progress_current, progress_total, message,
                  error_category AS "error_category: ErrorCategory", error_message, result,
                  created_at AS "created_at: OffsetDateTime",
                  started_at AS "started_at: OffsetDateTime",
                  completed_at AS "completed_at: OffsetDateTime"
             FROM tasks WHERE id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(Task::from))
}

/// Newest first. `owner = Some(id)` restricts to tasks that user created.
pub async fn list(
    db: &SqlitePool,
    owner: Option<Uuid>,
    status: Option<TaskStatus>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    let owner = owner.map(|o| o.hyphenated());
    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM tasks
            WHERE (?1 IS NULL OR created_by = ?1) AND (?2 IS NULL OR status = ?2)"#,
        owner,
        status
    )
    .fetch_one(db)
    .await?;
    let rows = sqlx::query_as!(
        TaskRow,
        r#"SELECT id AS "id: Hyphenated", kind, camera_id AS "camera_id: Hyphenated",
                  created_by AS "created_by: Hyphenated", created_by_username,
                  status AS "status: TaskStatus", progress_current, progress_total, message,
                  error_category AS "error_category: ErrorCategory", error_message, result,
                  created_at AS "created_at: OffsetDateTime",
                  started_at AS "started_at: OffsetDateTime",
                  completed_at AS "completed_at: OffsetDateTime"
             FROM tasks
            WHERE (?1 IS NULL OR created_by = ?1) AND (?2 IS NULL OR status = ?2)
            ORDER BY id DESC LIMIT ?3 OFFSET ?4"#,
        owner,
        status,
        limit,
        offset
    )
    .fetch_all(db)
    .await?;
    Ok((rows.into_iter().map(Task::from).collect(), total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn manager() -> (TaskManager, EventBus) {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let bus = EventBus::default();
        (TaskManager::new(db, bus.clone()), bus)
    }

    async fn wait_terminal(m: &TaskManager, id: Uuid) -> Task {
        for _ in 0..200 {
            let t = get(m.db(), id).await.unwrap().unwrap();
            if t.status.is_terminal() {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("task {id} did not finish");
    }

    #[tokio::test]
    async fn success_with_progress_and_events() {
        let (m, bus) = manager().await;
        let mut rx = bus.subscribe();
        let task = m
            .spawn("demo", None, None, |ctx| async move {
                ctx.progress(1, Some(2), "first step").await;
                ctx.progress(2, Some(2), "second step").await;
                Ok(serde_json::json!({ "answer": 42 }))
            })
            .await
            .unwrap();
        assert_eq!(task.status, TaskStatus::Queued);
        let done = wait_terminal(&m, task.id).await;
        assert_eq!(done.status, TaskStatus::Succeeded);
        assert_eq!(done.result.unwrap()["answer"], 42);
        assert_eq!((done.progress_current, done.progress_total), (2, Some(2)));
        assert!(done.started_at.is_some() && done.completed_at.is_some());

        let mut statuses = Vec::new();
        while let Ok(Event::TaskUpdated { task }) = rx.try_recv() {
            statuses.push(task.status);
        }
        assert_eq!(statuses.first(), Some(&TaskStatus::Queued));
        assert!(statuses.contains(&TaskStatus::Running));
        assert_eq!(statuses.last(), Some(&TaskStatus::Succeeded));
    }

    #[tokio::test]
    async fn failure_keeps_category_and_partial_result() {
        let (m, _) = manager().await;
        let task = m
            .spawn("demo", None, None, |_| async {
                Err(TaskFailure {
                    category: ErrorCategory::AuthFailed,
                    message: "rejected".into(),
                    result: Some(serde_json::json!({ "step": 2 })),
                })
            })
            .await
            .unwrap();
        let done = wait_terminal(&m, task.id).await;
        assert_eq!(done.status, TaskStatus::Failed);
        assert_eq!(done.error_category, Some(ErrorCategory::AuthFailed));
        assert_eq!(done.result.unwrap()["step"], 2);
    }

    #[tokio::test]
    async fn cancel_running_and_reject_finished() {
        let (m, _) = manager().await;
        let task = m
            .spawn("slow", None, None, |_| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(Value::Null)
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        m.cancel(task.id).await.unwrap();
        let done = wait_terminal(&m, task.id).await;
        assert_eq!(done.status, TaskStatus::Cancelled);
        assert!(matches!(
            m.cancel(task.id).await,
            Err(CancelError::AlreadyFinished)
        ));
        assert!(matches!(
            m.cancel(Uuid::now_v7()).await,
            Err(CancelError::NotFound)
        ));
    }

    #[tokio::test]
    async fn concurrency_is_bounded_and_extra_tasks_queue() {
        let (m, _) = manager().await;
        let gate = Arc::new(tokio::sync::Notify::new());
        let mut ids = Vec::new();
        for _ in 0..MAX_RUNNING + 2 {
            let gate = gate.clone();
            ids.push(
                m.spawn("blocked", None, None, move |_| async move {
                    gate.notified().await;
                    Ok(Value::Null)
                })
                .await
                .unwrap()
                .id,
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (running, _) = list(m.db(), None, Some(TaskStatus::Running), 50, 0)
            .await
            .unwrap();
        let (queued, _) = list(m.db(), None, Some(TaskStatus::Queued), 50, 0)
            .await
            .unwrap();
        assert_eq!(running.len(), MAX_RUNNING);
        assert_eq!(queued.len(), 2);
        // Cancelling a queued task works without it ever running.
        m.cancel(queued[0].id).await.unwrap();
        assert_eq!(
            wait_terminal(&m, queued[0].id).await.status,
            TaskStatus::Cancelled
        );
        for _ in 0..MAX_RUNNING + 2 {
            gate.notify_waiters();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for id in ids {
            assert!(wait_terminal(&m, id).await.status.is_terminal());
        }
    }

    #[tokio::test]
    async fn panics_become_internal_failures() {
        let (m, _) = manager().await;
        let task = m
            .spawn("boom", None, None, |_| async { panic!("boom") })
            .await
            .unwrap();
        let done = wait_terminal(&m, task.id).await;
        assert_eq!(done.status, TaskStatus::Failed);
        assert_eq!(done.error_category, Some(ErrorCategory::Internal));
    }

    #[tokio::test]
    async fn interrupted_tasks_are_marked_failed_on_startup() {
        let (m, _) = manager().await;
        sqlx::query(
            "INSERT INTO tasks (id, kind, status, created_at) VALUES ('t1', 'x', 'running', '2026-01-01T00:00:00Z'),
                                                                   ('t2', 'x', 'succeeded', '2026-01-01T00:00:00Z')",
        )
        .execute(m.db())
        .await
        .unwrap();
        assert_eq!(m.mark_interrupted().await.unwrap(), 1);
        let status: String = sqlx::query_scalar("SELECT status FROM tasks WHERE id = 't1'")
            .fetch_one(m.db())
            .await
            .unwrap();
        assert_eq!(status, "failed");
    }

    #[tokio::test]
    async fn list_filters_by_owner() {
        let (m, _) = manager().await;
        let db = m.db().clone();
        let alice = crate::users::create(&db, "alice", "h", crate::auth::Role::Viewer)
            .await
            .unwrap();
        let creator = Creator {
            id: alice.id,
            username: "alice".into(),
        };
        let mine = m
            .spawn("x", None, Some(creator), |_| async { Ok(Value::Null) })
            .await
            .unwrap();
        m.spawn("x", None, None, |_| async { Ok(Value::Null) })
            .await
            .unwrap();
        let (own, total) = list(&db, Some(alice.id), None, 50, 0).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(own[0].id, mine.id);
        assert_eq!(list(&db, None, None, 50, 0).await.unwrap().1, 2);
    }
}
