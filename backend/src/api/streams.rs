//! `/api/v1/streams/*`: live video egress (ADR 0002) and the admin streams
//! overview (SPEC §7.6).
//!
//! Media WebSocket protocol (`GET /api/v1/streams/{camera_id}/ws`):
//! - text `{"type":"init","codec":"avc1.…","width":…,"height":…}`, then one
//!   binary message with the fMP4 initialization segment (repeated whenever the
//!   codec parameters change);
//! - binary media messages: 1 flag byte (bit 0 = keyframe) followed by one
//!   fMP4 fragment (`moof` + `mdat`). The first media message is a keyframe;
//! - text `{"type":"state","state":"starting|running|reconnecting|draining|failed|stopped","message":…}`.
//!
//! The control socket (`/api/v1/ws`, ADR 0007) never carries media.

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{PathParam, QueryParams};
use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::auth::{csrf, AdminUser, AuthUser, ClientIp};
use crate::cameras;
use crate::providers::StreamProfileId;
use crate::streaming::manager::{Priority, StreamError};
use crate::streaming::profiles::{resolve_profile, ViewPurpose};
use crate::streaming::{Packet, StreamInfo, StreamKey};
use crate::AppState;

const HEARTBEAT: Duration = Duration::from_secs(20);

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StreamQuery {
    /// Stream profile name. Omit to let the server choose for `purpose`.
    pub profile: Option<String>,
    /// What the video is for when no profile is given: `detail` (largest
    /// resolution, default) or `grid` (smallest).
    #[param(inline)]
    pub purpose: Option<ViewPurpose>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StopQuery {
    /// Stream profile name (empty for the camera default stream).
    pub profile: String,
}

/// Watch a camera's live video.
///
/// Required role: viewer. Upgrades to the media WebSocket described in ADR 0002
/// (session cookie plus a same-origin `Origin` header). All viewers of the same
/// camera and profile share one upstream camera connection.
#[utoipa::path(
    get,
    path = "/streams/{camera_id}/ws",
    operation_id = "open_stream",
    tag = "streams",
    params(("camera_id" = Uuid, Path, description = "Camera ID"), StreamQuery),
    responses(
        (status = 101, description = "Switching protocols"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Origin not allowed", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
        (status = 409, description = "No playable stream profile, or stream refused", body = Problem),
    )
)]
pub async fn watch(
    _user: AuthUser,
    State(state): State<AppState>,
    PathParam(camera_id): PathParam<Uuid>,
    QueryParams(q): QueryParams<StreamQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !csrf::origin_allowed(
        &headers,
        state.config.tls_enabled(),
        &state.config.server.allowed_origins,
    ) {
        return Err(ApiError::CsrfRejected);
    }
    let camera = cameras::get(&state.db, camera_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let profile = match q.profile {
        Some(p) => StreamProfileId(p),
        None => {
            let purpose = q.purpose.unwrap_or_default();
            let streaming = state.settings.current().streaming;
            let (camera_choice, global_choice) = match purpose {
                ViewPurpose::Grid => (&camera.grid_profile, &streaming.default_grid_profile),
                ViewPurpose::Detail => (&camera.detail_profile, &streaming.default_detail_profile),
            };
            resolve_profile(
                &camera.stream_profiles,
                purpose,
                camera_choice.as_deref(),
                global_choice.as_deref(),
            )
            .ok_or_else(|| {
            ApiError::Conflict(
                "This camera has no playable (H.264) stream profile yet. Try again after its details load."
                    .into(),
            )
            })?
        }
    };
    let priority = if q.purpose == Some(ViewPurpose::Grid) {
        Priority::Grid
    } else {
        Priority::Detail
    };
    let sub = match state
        .streams
        .subscribe_with(StreamKey { camera_id, profile }, priority)
    {
        Ok(sub) => sub,
        // A limit refused the stream: accept the socket so the viewer can be
        // told why (browsers hide handshake status codes), then close it.
        Err(StreamError::Refused(reason)) => {
            return Ok(upgrade.on_upgrade(move |mut socket| async move {
                let msg = text(
                    serde_json::json!({ "type": "state", "state": "failed", "message": reason }),
                );
                let _ = socket.send(msg).await;
                let _ = socket.send(Message::Close(None)).await;
            }));
        }
        Err(StreamError::ShuttingDown) => {
            return Err(ApiError::Unavailable("the server is shutting down".into()))
        }
    };
    Ok(upgrade.on_upgrade(move |socket| pump(socket, sub)))
}

fn text(v: serde_json::Value) -> Message {
    Message::Text(Utf8Bytes::from(v.to_string()))
}

async fn pump(socket: WebSocket, mut sub: crate::streaming::manager::Subscription) {
    let (mut tx, mut rx) = socket.split();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.reset();
    loop {
        let msg = tokio::select! {
            packet = sub.next() => match packet {
                Some(Packet::Init(init)) => {
                    let hello = text(serde_json::json!({
                        "type": "init", "codec": init.codec,
                        "width": init.width, "height": init.height,
                    }));
                    if tx.send(hello).await.is_err() {
                        return;
                    }
                    Message::Binary(init.data.clone())
                }
                Some(Packet::Fragment(f)) => {
                    let mut b = BytesMut::with_capacity(1 + f.data.len());
                    b.put_u8(u8::from(f.keyframe));
                    b.extend_from_slice(&f.data);
                    Message::Binary(b.freeze())
                }
                Some(Packet::State { state, message }) => {
                    text(serde_json::json!({ "type": "state", "state": state, "message": message }))
                }
                None => {
                    let _ = tx
                        .send(Message::Close(Some(CloseFrame {
                            code: 1000,
                            reason: Utf8Bytes::from_static("stream ended"),
                        })))
                        .await;
                    return;
                }
            },
            incoming = rx.next() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => continue, // pongs and anything else are ignored
            },
            _ = heartbeat.tick() => Message::Ping(Bytes::new()),
        };
        if tx.send(msg).await.is_err() {
            return; // viewer went away; dropping `sub` unsubscribes
        }
    }
}

/// List active streams with their metrics.
///
/// Required role: admin. Live updates of the same data are on the `streams`
/// WebSocket topic.
#[utoipa::path(
    get,
    path = "/streams",
    operation_id = "list_streams",
    tag = "streams",
    responses(
        (status = 200, description = "Active streams", body = Vec<StreamInfo>),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn list(_admin: AdminUser, State(state): State<AppState>) -> Json<Vec<StreamInfo>> {
    Json(state.streams.list())
}

/// Force-stop a stream, disconnecting its viewers.
///
/// Required role: admin. Audited. Viewers can reconnect, which starts a new
/// upstream connection.
#[utoipa::path(
    delete,
    path = "/streams/{camera_id}",
    operation_id = "stop_stream",
    tag = "streams",
    params(("camera_id" = Uuid, Path, description = "Camera ID"), StopQuery),
    responses(
        (status = 204, description = "Stopped"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such active stream", body = Problem),
    )
)]
pub async fn stop(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(camera_id): PathParam<Uuid>,
    QueryParams(q): QueryParams<StopQuery>,
) -> Result<StatusCode, ApiError> {
    let key = StreamKey {
        camera_id,
        profile: StreamProfileId(q.profile.clone()),
    };
    if !state.streams.stop(&key) {
        return Err(ApiError::NotFound);
    }
    audit::record(
        &state.db,
        AuditEvent::new(action::STREAM_FORCE_STOP, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("camera", camera_id)
            .ip(ip)
            .details(serde_json::json!({ "profile": q.profile })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
