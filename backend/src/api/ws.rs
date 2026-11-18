//! `/api/v1/ws`: the multiplexed real-time connection (SPEC §13.2, ADR 0007).
//!
//! - Authenticated with the session cookie; `Origin` must match the server.
//! - Clients send `subscribe` / `unsubscribe` with topics; the server forwards
//!   matching events the user may see.
//! - The server pings every [`HEARTBEAT`]; a connection that hasn't answered for
//!   three heartbeats is closed (4408). The session is re-validated every
//!   `auth.websocket_recheck_secs` (default 60);
//!   when it ends the socket closes with 4401 and the client should sign in again.
//! - If a client falls too far behind the event stream it gets `resync` and
//!   should refetch state over REST.
//! - Media is never sent here; it has its own endpoints (ADR 0002).

use std::collections::BTreeSet;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum_extra::extract::CookieJar;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use crate::auth::session::{self, Timeouts, COOKIE_NAME};
use crate::auth::{csrf, AuthUser, Role};
use crate::cameras::{CameraStatusView, CameraSummary};
use crate::events::{Event, Topic};
use crate::tasks::Task;
use crate::AppState;

pub const HEARTBEAT: Duration = Duration::from_secs(20);
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

pub const CLOSE_SESSION_ENDED: u16 = 4401;
pub const CLOSE_HEARTBEAT_TIMEOUT: u16 = 4408;

/// Messages a client may send.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMessage {
    Subscribe { topics: Vec<Topic> },
    Unsubscribe { topics: Vec<Topic> },
}

/// Messages the server sends.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMessage {
    /// First message after connecting.
    Hello {
        username: String,
        role: Role,
        heartbeat_secs: u64,
    },
    /// The full set of active topics after a (un)subscribe.
    Subscribed {
        topics: Vec<Topic>,
    },
    /// The client missed events; refetch state over REST.
    Resync {
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
    CameraStatus {
        camera_id: Uuid,
        status: CameraStatusView,
    },
    CameraChanged {
        camera: CameraSummary,
    },
    CameraDeleted {
        camera_id: Uuid,
    },
    /// A background task changed state or reported progress.
    TaskUpdated {
        task: Task,
    },
}

impl From<Event> for WsServerMessage {
    fn from(e: Event) -> Self {
        match e {
            Event::CameraStatus { camera_id, status } => {
                WsServerMessage::CameraStatus { camera_id, status }
            }
            Event::CameraChanged { camera } => WsServerMessage::CameraChanged { camera },
            Event::CameraDeleted { camera_id } => WsServerMessage::CameraDeleted { camera_id },
            Event::TaskUpdated { task } => WsServerMessage::TaskUpdated { task },
        }
    }
}

/// Authorization hook for events. In the MVP every signed-in user sees every
/// camera; per-group access (SPEC §18 Q4) plugs in here. Tasks are visible to
/// the user who started them, and to admins.
fn may_see(user: &AuthUser, event: &Event) -> bool {
    match event {
        Event::TaskUpdated { task } => user.is_admin() || task.created_by == Some(user.id),
        _ => true,
    }
}

/// Open the real-time WebSocket.
///
/// Required role: viewer. Upgrade with the session cookie and a same-origin
/// `Origin` header. Messages are JSON objects with a `type` field; see the
/// `WsClientMessage` and `WsServerMessage` schemas.
#[utoipa::path(
    get,
    path = "/ws",
    operation_id = "open_websocket",
    tag = "realtime",
    responses(
        (status = 101, description = "Switching protocols"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Origin not allowed", body = Problem),
    )
)]
pub async fn ws(
    user: AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !csrf::origin_allowed(
        &headers,
        state.config.tls_enabled(),
        &state.config.server.allowed_origins,
    ) {
        return Err(ApiError::CsrfRejected);
    }
    let token = jar
        .get(COOKIE_NAME)
        .map(|c| c.value().to_owned())
        .ok_or(ApiError::Unauthorized)?;
    Ok(upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| run(socket, state, user, token)))
}

fn json(msg: &WsServerMessage) -> Message {
    // Serializing these plain data types can't fail.
    Message::Text(Utf8Bytes::from(
        serde_json::to_string(msg).unwrap_or_default(),
    ))
}

fn close(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: Utf8Bytes::from_static(reason),
    }))
}

async fn run(socket: WebSocket, state: AppState, user: AuthUser, token: String) {
    let (mut tx, mut rx) = socket.split();
    let mut events = state.events.subscribe();
    let mut topics: BTreeSet<Topic> = BTreeSet::new();
    let timeouts = Timeouts::from(&state.config.auth);

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.reset(); // first tick after one interval
    let mut session_check = tokio::time::interval(Duration::from_secs(
        state.config.auth.websocket_recheck_secs,
    ));
    session_check.reset();
    let mut last_seen = Instant::now();

    let hello = WsServerMessage::Hello {
        username: user.username.clone(),
        role: user.role,
        heartbeat_secs: HEARTBEAT.as_secs(),
    };
    if tx.send(json(&hello)).await.is_err() {
        return;
    }
    tracing::debug!(user = %user.username, "websocket connected");

    loop {
        let outgoing = tokio::select! {
            incoming = rx.next() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Text(text))) => {
                    last_seen = Instant::now();
                    Some(handle_client(&text, &mut topics))
                }
                Some(Ok(Message::Pong(_) | Message::Ping(_))) => {
                    last_seen = Instant::now();
                    None
                }
                Some(Ok(Message::Binary(_))) => Some(WsServerMessage::Error {
                    code: "bad_message".into(),
                    message: "binary messages are not supported".into(),
                }),
            },
            event = events.recv() => match event {
                Ok(event) if topics.contains(&event.topic()) && may_see(&user, &event) => {
                    Some(WsServerMessage::from(event))
                }
                Ok(_) => None,
                Err(RecvError::Lagged(skipped)) => Some(WsServerMessage::Resync {
                    reason: format!("missed {skipped} events"),
                }),
                Err(RecvError::Closed) => break,
            },
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > HEARTBEAT * 3 {
                    let _ = tx.send(close(CLOSE_HEARTBEAT_TIMEOUT, "heartbeat timeout")).await;
                    break;
                }
                if tx.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
                None
            }
            _ = session_check.tick() => {
                match session::validate(&state.db, &token, timeouts).await {
                    Ok(Some(s)) if s.role == user.role => None,
                    // Ended, expired, disabled, or role changed: make the client reconnect
                    // so authorization is re-evaluated.
                    Ok(_) => {
                        let _ = tx.send(close(CLOSE_SESSION_ENDED, "session ended")).await;
                        break;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "websocket session check failed");
                        None
                    }
                }
            }
        };
        if let Some(msg) = outgoing {
            if tx.send(json(&msg)).await.is_err() {
                break;
            }
        }
    }
    tracing::debug!(user = %user.username, "websocket disconnected");
}

fn handle_client(text: &str, topics: &mut BTreeSet<Topic>) -> WsServerMessage {
    match serde_json::from_str::<WsClientMessage>(text) {
        Ok(WsClientMessage::Subscribe { topics: add }) => {
            topics.extend(add);
            WsServerMessage::Subscribed {
                topics: topics.iter().copied().collect(),
            }
        }
        Ok(WsClientMessage::Unsubscribe { topics: remove }) => {
            for t in remove {
                topics.remove(&t);
            }
            WsServerMessage::Subscribed {
                topics: topics.iter().copied().collect(),
            }
        }
        Err(e) => WsServerMessage::Error {
            code: "bad_message".into(),
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_unsubscribe() {
        let mut topics = BTreeSet::new();
        let reply = handle_client(
            r#"{"type":"subscribe","topics":["camera_status","cameras"]}"#,
            &mut topics,
        );
        assert_eq!(
            reply,
            WsServerMessage::Subscribed {
                topics: vec![Topic::CameraStatus, Topic::Cameras]
            }
        );
        let reply = handle_client(
            r#"{"type":"unsubscribe","topics":["cameras"]}"#,
            &mut topics,
        );
        assert_eq!(
            reply,
            WsServerMessage::Subscribed {
                topics: vec![Topic::CameraStatus]
            }
        );
    }

    #[test]
    fn unknown_messages_and_topics_are_errors() {
        let mut topics = BTreeSet::new();
        for bad in [
            "not json",
            r#"{"type":"explode"}"#,
            r#"{"type":"subscribe","topics":["secrets"]}"#,
        ] {
            assert!(
                matches!(
                    handle_client(bad, &mut topics),
                    WsServerMessage::Error { .. }
                ),
                "{bad}"
            );
        }
        assert!(topics.is_empty());
    }
}
