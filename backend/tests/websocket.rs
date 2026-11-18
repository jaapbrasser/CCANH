//! WebSocket over a real TCP connection: auth, origin check, subscriptions,
//! event delivery and session expiry.

mod common;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crane::auth::session::{self, Timeouts};
use crane::auth::Role;
use crane::events::Event;

struct Server {
    app: common::TestApp,
    addr: std::net::SocketAddr,
    shutdown: tokio_util::sync::CancellationToken,
}

async fn start() -> Server {
    let mut config = crane::config::Config::default();
    config.auth.websocket_recheck_secs = 1;
    let app = common::TestApp::with_config(config).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    tokio::spawn(crane::serve(
        listener,
        app.state.clone(),
        None,
        shutdown.clone(),
    ));
    Server {
        app,
        addr,
        shutdown,
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn cookie_for(server: &Server, username: &str) -> String {
    let user = server.app.user(username, Role::Viewer).await;
    let token = session::create(
        &server.app.state.db,
        user.id,
        None,
        None,
        Timeouts::from(&server.app.state.config.auth),
    )
    .await
    .unwrap();
    format!("crane_session={token}")
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(
    server: &Server,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{}/api/v1/ws", server.addr)
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert(
        "host",
        HeaderValue::from_str(&server.addr.to_string()).unwrap(),
    );
    if let Some(c) = cookie {
        headers.insert("cookie", HeaderValue::from_str(c).unwrap());
    }
    if let Some(o) = origin {
        headers.insert("origin", HeaderValue::from_str(o).unwrap());
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(s, _)| s)
}

async fn next_json(socket: &mut Socket) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("no message within 5s")
            .expect("socket closed")
            .expect("socket error");
        match msg {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected message: {other:?}"),
        }
    }
}

fn status_code(err: tokio_tungstenite::tungstenite::Error) -> u16 {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => resp.status().as_u16(),
        other => panic!("expected an HTTP rejection, got {other}"),
    }
}

#[tokio::test]
async fn requires_session_and_matching_origin() {
    let server = start().await;
    let origin = format!("http://{}", server.addr);
    let anon = connect(&server, None, Some(&origin)).await.unwrap_err();
    assert_eq!(status_code(anon), 401);

    let cookie = cookie_for(&server, "alice").await;
    let foreign = connect(&server, Some(&cookie), Some("http://evil.example"))
        .await
        .unwrap_err();
    assert_eq!(status_code(foreign), 403);
    let missing = connect(&server, Some(&cookie), None).await.unwrap_err();
    assert_eq!(status_code(missing), 403);
}

#[tokio::test]
async fn subscribes_and_receives_only_subscribed_events() {
    let server = start().await;
    let cookie = cookie_for(&server, "alice").await;
    let origin = format!("http://{}", server.addr);
    let mut socket = connect(&server, Some(&cookie), Some(&origin))
        .await
        .unwrap();

    let hello = next_json(&mut socket).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["username"], "alice");
    assert_eq!(hello["role"], "viewer");

    // Not subscribed yet: events are not delivered.
    server.app.state.events.publish(Event::CameraDeleted {
        camera_id: Uuid::now_v7(),
    });

    socket
        .send(Message::Text(
            json!({ "type": "subscribe", "topics": ["cameras"] })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let subscribed = next_json(&mut socket).await;
    assert_eq!(
        subscribed,
        json!({ "type": "subscribed", "topics": ["cameras"] })
    );

    let id = Uuid::now_v7();
    server
        .app
        .state
        .events
        .publish(Event::CameraDeleted { camera_id: id });
    let event = next_json(&mut socket).await;
    assert_eq!(event["type"], "camera_deleted");
    assert_eq!(event["camera_id"], id.to_string());

    socket
        .send(Message::Text(
            r#"{"type":"subscribe","topics":["nope"]}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(next_json(&mut socket).await["type"], "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn closes_when_session_ends() {
    let server = start().await;
    let cookie = cookie_for(&server, "alice").await;
    let origin = format!("http://{}", server.addr);
    let mut socket = connect(&server, Some(&cookie), Some(&origin))
        .await
        .unwrap();
    assert_eq!(next_json(&mut socket).await["type"], "hello");

    sqlx::query("DELETE FROM sessions")
        .execute(&server.app.state.db)
        .await
        .unwrap();

    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(msg) = socket.next().await {
            if let Ok(Message::Close(Some(frame))) = msg {
                return Some(u16::from(frame.code));
            }
        }
        None
    })
    .await
    .expect("socket was not closed after the session ended");
    assert_eq!(closed, Some(crane::api::ws::CLOSE_SESSION_ENDED));
}

#[tokio::test]
async fn task_updates_only_reach_their_owner() {
    let server = start().await;
    let cookie = cookie_for(&server, "alice").await;
    let alice = crane::users::find_credentials(&server.app.state.db, "alice")
        .await
        .unwrap()
        .unwrap()
        .user;
    let origin = format!("http://{}", server.addr);
    let mut socket = connect(&server, Some(&cookie), Some(&origin))
        .await
        .unwrap();
    assert_eq!(next_json(&mut socket).await["type"], "hello");
    socket
        .send(Message::Text(
            json!({ "type": "subscribe", "topics": ["tasks"] })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(next_json(&mut socket).await["type"], "subscribed");

    // Someone else's task: never delivered. Alice's own task: delivered.
    let tasks = &server.app.state.tasks;
    tasks
        .spawn("other", None, None, |_| async {
            Ok(serde_json::Value::Null)
        })
        .await
        .unwrap();
    let mine = tasks
        .spawn(
            "mine",
            None,
            Some(crane::tasks::Creator {
                id: alice.id,
                username: "alice".into(),
            }),
            |_| async { Ok(serde_json::Value::Null) },
        )
        .await
        .unwrap();

    let first = next_json(&mut socket).await;
    assert_eq!(first["type"], "task_updated");
    assert_eq!(
        first["task"]["id"],
        mine.id.to_string(),
        "only own tasks: {first}"
    );
    loop {
        let msg = next_json(&mut socket).await;
        assert_eq!(msg["task"]["kind"], "mine", "{msg}");
        if msg["task"]["status"] == "succeeded" {
            break;
        }
    }
}
