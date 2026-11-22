//! Pan/tilt/zoom for one camera or every PTZ camera on a dashboard (issue
//! #31, ADR 0013).

use axum::extract::State;
use axum::Json;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{JsonBody, PathParam};
use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::auth::{AuthUser, ClientIp};
use crate::cameras;
use crate::providers::PtzMove;
use crate::ptz::{self, PtzOutcome, Target};
use crate::AppState;

fn check(user: &AuthUser, state: &AppState, mv: &PtzMove) -> Result<(), ApiError> {
    if !mv.in_range() {
        return Err(ApiError::BadRequest(
            "pan, tilt and zoom must each be between -100 and 100".into(),
        ));
    }
    state
        .ptz
        .admit(user.id)
        .map_err(|retry_after_secs| ApiError::TooManyRequests { retry_after_secs })
}

/// Providers for the PTZ-capable cameras among `ids`, with their names.
async fn targets(
    state: &AppState,
    ids: &[Uuid],
) -> Result<(Vec<Target>, Vec<serde_json::Value>), ApiError> {
    let mut targets = Vec::new();
    let mut names = Vec::new();
    for &id in ids {
        let Some(camera) = cameras::get(&state.db, id).await? else {
            continue;
        };
        if !camera.summary.ptz {
            continue;
        }
        let Some(target) = cameras::target(&state.db, id).await? else {
            continue;
        };
        let target = target
            .provider_target(&*state.credentials)
            .map_err(|e| ApiError::Internal(format!("cannot decrypt camera credentials: {e}")))?;
        names.push(serde_json::json!({ "id": id, "name": camera.summary.name }));
        targets.push(Target {
            camera_id: id,
            provider: state.providers().create(&target),
        });
    }
    Ok((targets, names))
}

/// One audit entry per gesture: who, which cameras, what.
async fn audit_gesture(
    state: &AppState,
    user: &AuthUser,
    ip: Option<std::net::IpAddr>,
    target: (&'static str, Uuid),
    cameras: Vec<serde_json::Value>,
    mv: PtzMove,
    out: &PtzOutcome,
) -> Result<(), ApiError> {
    if !out.new_gesture {
        return Ok(());
    }
    let failed = out.results.iter().filter(|r| !r.ok).count();
    audit::record(
        &state.db,
        AuditEvent::new(
            action::CAMERA_PTZ,
            if failed == out.results.len() {
                Outcome::Failure
            } else {
                Outcome::Success
            },
        )
        .actor(Some(user.id), &user.username)
        .target(target.0, target.1)
        .ip(ip)
        .details(serde_json::json!({
            "action": ptz::describe(mv),
            "move": mv,
            "cameras": cameras,
            "failed": failed,
        })),
    )
    .await?;
    Ok(())
}

/// Move one camera: start, change or stop continuous pan/tilt/zoom.
///
/// Required role: viewer. Send the move when a control is pressed, repeat it
/// about every 400 ms while it's held, and send all zeros on release; a move
/// not repeated within 1.5 s is stopped by the server. Each new or changed
/// move is audited once (`camera.ptz`); repeats and stops aren't. 400 if the
/// camera can't pan, tilt or zoom; 429 when sending too fast.
#[utoipa::path(
    post,
    path = "/cameras/{id}/ptz",
    operation_id = "move_camera",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID")),
    request_body = PtzMove,
    responses(
        (status = 200, description = "Per-camera result", body = PtzOutcome),
        (status = 400, description = "Not a PTZ camera, or speeds out of range", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
        (status = 429, description = "Too many commands", body = Problem),
    )
)]
pub async fn move_camera(
    user: AuthUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(mv): JsonBody<PtzMove>,
) -> Result<Json<PtzOutcome>, ApiError> {
    check(&user, &state, &mv)?;
    if cameras::get(&state.db, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let (targets, names) = targets(&state, &[id]).await?;
    if targets.is_empty() {
        return Err(ApiError::BadRequest(
            "this camera can't pan, tilt or zoom".into(),
        ));
    }
    let out = state.ptz.command(targets, mv).await;
    audit_gesture(&state, &user, ip, ("camera", id), names, mv, &out).await?;
    Ok(Json(out))
}

/// Move every PTZ camera on one of your dashboards at once.
///
/// Required role: viewer; only your own dashboards. Same protocol as
/// `move_camera`; cameras without PTZ are left out, and each gesture is one
/// audit entry listing the cameras. `results` is empty when the dashboard
/// has no PTZ camera.
#[utoipa::path(
    post,
    path = "/dashboards/{id}/ptz",
    operation_id = "move_dashboard_cameras",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Dashboard ID")),
    request_body = PtzMove,
    responses(
        (status = 200, description = "Per-camera result", body = PtzOutcome),
        (status = 400, description = "Speeds out of range", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such dashboard of yours", body = Problem),
        (status = 429, description = "Too many commands", body = Problem),
    )
)]
pub async fn move_dashboard(
    user: AuthUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(mv): JsonBody<PtzMove>,
) -> Result<Json<PtzOutcome>, ApiError> {
    check(&user, &state, &mv)?;
    let dashboard = crate::dashboards::get(&state.db, user.id, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let ids: Vec<Uuid> = dashboard.tiles.iter().map(|t| t.camera_id).collect();
    let (targets, names) = targets(&state, &ids).await?;
    if targets.is_empty() {
        return Ok(Json(PtzOutcome {
            results: Vec::new(),
            new_gesture: false,
        }));
    }
    let out = state.ptz.command(targets, mv).await;
    audit_gesture(&state, &user, ip, ("dashboard", id), names, mv, &out).await?;
    Ok(Json(out))
}
