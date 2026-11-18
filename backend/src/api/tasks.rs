//! `/api/v1/tasks/*`: background task status and cancellation (SPEC §10.3).
//!
//! Users see the tasks they started; admins see all. Someone else's task is a
//! 404 for non-admins, so task ids reveal nothing.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{PathParam, QueryParams};
use super::pagination::{Page, PageParams};
use crate::auth::AuthUser;
use crate::tasks::{self, CancelError, Task, TaskStatus};
use crate::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TaskListQuery {
    #[param(minimum = 1)]
    pub page: Option<u32>,
    #[param(minimum = 1, maximum = 200)]
    pub page_size: Option<u32>,
    #[param(inline)]
    pub status: Option<TaskStatus>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskList {
    pub items: Vec<Task>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

fn visible(user: &AuthUser, task: &Task) -> bool {
    user.is_admin() || task.created_by == Some(user.id)
}

/// List background tasks, newest first.
///
/// Required role: viewer. Viewers see their own tasks; admins see all.
#[utoipa::path(
    get,
    path = "/tasks",
    operation_id = "list_tasks",
    tag = "tasks",
    params(TaskListQuery),
    responses(
        (status = 200, description = "A page of tasks", body = TaskList),
        (status = 400, description = "Invalid query", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn list(
    user: AuthUser,
    State(state): State<AppState>,
    QueryParams(q): QueryParams<TaskListQuery>,
) -> Result<Json<TaskList>, ApiError> {
    let page = Page::try_from(PageParams {
        page: q.page,
        page_size: q.page_size,
    })?;
    let owner = (!user.is_admin()).then_some(user.id);
    let (items, total) =
        tasks::list(&state.db, owner, q.status, page.limit(), page.offset()).await?;
    Ok(Json(TaskList {
        items,
        total,
        page: page.page,
        page_size: page.page_size,
    }))
}

/// Get one task, including its result once finished.
///
/// Required role: viewer. Only the task's creator or an admin can see it.
#[utoipa::path(
    get,
    path = "/tasks/{id}",
    operation_id = "get_task",
    tag = "tasks",
    params(("id" = Uuid, Path, description = "Task ID")),
    responses(
        (status = 200, description = "The task", body = Task),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such task (or not yours)", body = Problem),
    )
)]
pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<Task>, ApiError> {
    match tasks::get(&state.db, id).await? {
        Some(t) if visible(&user, &t) => Ok(Json(t)),
        _ => Err(ApiError::NotFound),
    }
}

/// Cancel a queued or running task.
///
/// Required role: viewer. Only the task's creator or an admin can cancel it.
/// Returns 202: the task reports `cancelled` once it has stopped.
#[utoipa::path(
    post,
    path = "/tasks/{id}/cancel",
    operation_id = "cancel_task",
    tag = "tasks",
    params(("id" = Uuid, Path, description = "Task ID")),
    responses(
        (status = 202, description = "Cancellation requested"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such task (or not yours)", body = Problem),
        (status = 409, description = "Task already finished", body = Problem),
    )
)]
pub async fn cancel(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    match tasks::get(&state.db, id).await? {
        Some(t) if visible(&user, &t) => {}
        _ => return Err(ApiError::NotFound),
    }
    match state.tasks.cancel(id).await {
        Ok(()) => Ok(StatusCode::ACCEPTED),
        Err(CancelError::NotFound) => Err(ApiError::NotFound),
        Err(CancelError::AlreadyFinished) => {
            Err(ApiError::Conflict("This task has already finished.".into()))
        }
        Err(CancelError::Database(e)) => Err(e.into()),
    }
}
