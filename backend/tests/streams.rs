//! Live video end to end in demo mode: media WebSocket protocol, shared
//! upstream (Phase 5 acceptance: several viewers → one upstream), streams
//! overview and force-stop.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use crane::config::{Config, Mode};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Server {
    app: TestApp,
    addr: std::net::SocketAddr,
    shutdown: tokio_util::sync::CancellationToken,
    admin: String,
    camera: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn start() -> Server {
    let app = TestApp::with_config(Config {
        mode: Mode::Demo,
        ..Config::default()
    })
    .await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let camera = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "A1 Crane 1 – Hook", "host": "192.0.2.11", "monitoring_enabled": false
                })),
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Load stream profiles (main 1080p, grid 360p for this model).
    let refreshed = app
        .send(req(Method::POST, &format!("/api/v1/cameras/{camera}/refresh")).cookie(&admin))
        .await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.text());

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
        admin,
        camera,
    }
}

async fn connect(s: &Server, cookie: &str, query: &str, origin: bool) -> Result<Socket, u16> {
    let mut request = format!("ws://{}/api/v1/streams/{}/ws{query}", s.addr, s.camera)
        .into_client_request()
        .unwrap();
    let h = request.headers_mut();
    h.insert("cookie", HeaderValue::from_str(cookie).unwrap());
    if origin {
        h.insert(
            "origin",
            HeaderValue::from_str(&format!("http://{}", s.addr)).unwrap(),
        );
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(sock, _)| sock)
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r) => r.status().as_u16(),
            other => panic!("unexpected error {other}"),
        })
}

async fn next(sock: &mut Socket) -> Message {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), sock.next())
            .await
            .expect("no message within 10 s")
            .expect("socket closed")
            .expect("socket error");
        if !matches!(m, Message::Ping(_) | Message::Pong(_)) {
            return m;
        }
    }
}

/// Read until the init text + binary init + first media message.
async fn handshake(sock: &mut Socket) -> (Value, Vec<u8>, Vec<u8>) {
    let mut init_json = None;
    let mut init = None;
    loop {
        match next(sock).await {
            Message::Text(t) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                if v["type"] == "init" {
                    init_json = Some(v);
                }
            }
            Message::Binary(b) if init.is_none() => init = Some(b.to_vec()),
            Message::Binary(b) => return (init_json.unwrap(), init.unwrap(), b.to_vec()),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn three_viewers_share_one_upstream() {
    let s = start().await;
    let (_, viewer) = s.app.signed_in("viewer", Role::Viewer).await;
    let mut socks = Vec::new();
    for cookie in [&s.admin, &viewer, &viewer] {
        let mut sock = connect(&s, cookie, "?profile=grid", true).await.unwrap();
        let (meta, init, first) = handshake(&mut sock).await;
        assert!(
            meta["codec"].as_str().unwrap().starts_with("avc1.4D40"),
            "{meta}"
        );
        assert_eq!(meta["width"], 640);
        assert_eq!(&init[4..8], b"ftyp", "binary init is an fMP4 init segment");
        assert_eq!(first[0], 1, "first media message is a keyframe");
        assert_eq!(&first[5..9], b"moof");
        socks.push(sock);
    }

    let streams = s
        .app
        .send(req(Method::GET, "/api/v1/streams").cookie(&s.admin))
        .await;
    assert_eq!(streams.status, StatusCode::OK);
    let list = streams.json();
    assert_eq!(list.as_array().unwrap().len(), 1, "one upstream: {list}");
    assert_eq!(list[0]["subscribers"], 3);
    assert_eq!(list[0]["state"], "running");
    assert_eq!(list[0]["profile"], "grid");
    assert!(list[0]["bytes_in_total"].as_u64().unwrap() > 0);

    // Viewers can't see the overview; admins can force-stop.
    assert_eq!(
        s.app
            .send(req(Method::GET, "/api/v1/streams").cookie(&viewer))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let stop = s
        .app
        .send(
            req(
                Method::DELETE,
                &format!("/api/v1/streams/{}?profile=grid", s.camera),
            )
            .cookie(&s.admin),
        )
        .await;
    assert_eq!(stop.status, StatusCode::NO_CONTENT);
    for sock in &mut socks {
        let mut saw_stopped = false;
        loop {
            match tokio::time::timeout(Duration::from_secs(5), sock.next())
                .await
                .unwrap()
            {
                Some(Ok(Message::Text(t))) if t.contains("\"stopped\"") => saw_stopped = true,
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            }
        }
        assert!(saw_stopped, "viewers are told the stream stopped");
    }
    let audit = s
        .app
        .send(req(Method::GET, "/api/v1/audit?action=stream.force_stop").cookie(&s.admin))
        .await;
    assert_eq!(audit.json()["total"], 1);
}

#[tokio::test]
async fn default_profile_follows_purpose() {
    let s = start().await;
    let mut detail = connect(&s, &s.admin, "", true).await.unwrap();
    assert_eq!(
        handshake(&mut detail).await.0["width"],
        1280,
        "detail picks the largest"
    );
    let mut grid = connect(&s, &s.admin, "?purpose=grid", true).await.unwrap();
    assert_eq!(
        handshake(&mut grid).await.0["width"],
        640,
        "grid picks the smallest"
    );
    assert_eq!(
        s.app.state.streams.list().len(),
        2,
        "different profiles, different upstreams"
    );
}

#[tokio::test]
async fn media_socket_requires_origin_and_known_camera() {
    let s = start().await;
    assert_eq!(connect(&s, &s.admin, "", false).await.err(), Some(403));
    assert_eq!(
        connect(&s, "crane_session=nope", "", true).await.err(),
        Some(401)
    );
    let mut request = format!("ws://{}/api/v1/streams/{}/ws", s.addr, uuid::Uuid::now_v7())
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", HeaderValue::from_str(&s.admin).unwrap());
    request.headers_mut().insert(
        "origin",
        HeaderValue::from_str(&format!("http://{}", s.addr)).unwrap(),
    );
    let err = tokio_tungstenite::connect_async(request).await.unwrap_err();
    assert!(matches!(err, tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 404));
}

#[tokio::test]
async fn unsupported_profile_reports_failed() {
    let s = start().await;
    // A profile this camera doesn't have fails at once instead of retrying.
    let mut sock = connect(&s, &s.admin, "?profile=no-such-profile", true)
        .await
        .unwrap();
    let mut failed = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), sock.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Text(t))) if t.contains("\"failed\"") => failed = true,
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => {}
        }
    }
    assert!(failed);
}

#[tokio::test]
async fn camera_and_global_profile_choices_pick_the_stream() {
    let s = start().await;
    // Global default: the detail view uses "grid" on cameras that have it.
    let patched = s
        .app
        .send(
            req(Method::PATCH, "/api/v1/settings/streaming")
                .cookie(&s.admin)
                .json(json!({ "default_detail_profile": "grid" })),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());
    let mut detail = connect(&s, &s.admin, "", true).await.unwrap();
    assert_eq!(handshake(&mut detail).await.0["width"], 640);

    // The camera's own choice wins over the global default.
    let set = s
        .app
        .send(
            req(
                Method::PUT,
                &format!("/api/v1/cameras/{}/stream-profiles", s.camera),
            )
            .cookie(&s.admin)
            .json(json!({ "grid_profile": "main", "detail_profile": "main" })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text());
    assert_eq!(set.json()["grid_profile"], "main");
    let mut grid = connect(&s, &s.admin, "?purpose=grid", true).await.unwrap();
    assert_eq!(handshake(&mut grid).await.0["width"], 1280);

    // Only the camera's own playable profiles are accepted; null clears.
    let bad = s
        .app
        .send(
            req(
                Method::PUT,
                &format!("/api/v1/cameras/{}/stream-profiles", s.camera),
            )
            .cookie(&s.admin)
            .json(json!({ "grid_profile": "nope", "detail_profile": null })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let cleared = s
        .app
        .send(
            req(
                Method::PUT,
                &format!("/api/v1/cameras/{}/stream-profiles", s.camera),
            )
            .cookie(&s.admin)
            .json(json!({ "grid_profile": null, "detail_profile": null })),
        )
        .await;
    assert_eq!(cleared.json()["grid_profile"], Value::Null);

    let audit = s
        .app
        .send(req(Method::GET, "/api/v1/audit?action=camera.update").cookie(&s.admin))
        .await;
    assert_eq!(audit.json()["total"], 2, "both changes are audited");
}

#[tokio::test]
async fn removing_a_camera_stops_its_streams() {
    let s = start().await;
    let mut sock = connect(&s, &s.admin, "?profile=grid", true).await.unwrap();
    handshake(&mut sock).await;
    assert_eq!(s.app.state.streams.list().len(), 1);

    let del = s
        .app
        .send(req(Method::DELETE, &format!("/api/v1/cameras/{}", s.camera)).cookie(&s.admin))
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    let mut saw_stopped = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(5), sock.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Text(t))) if t.contains("camera was removed") => saw_stopped = true,
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => {}
        }
    }
    assert!(saw_stopped, "viewers are told why the stream ended");
    assert!(s.app.state.streams.list().is_empty());
}

#[tokio::test]
async fn connection_limits_refuse_new_upstreams_but_not_new_viewers() {
    let s = start().await;
    let patched = s
        .app
        .send(
            req(Method::PATCH, "/api/v1/settings/streaming")
                .cookie(&s.admin)
                .json(json!({ "max_upstreams": 1, "detail_reserve": 0 })),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text());

    let mut first = connect(&s, &s.admin, "?profile=grid", true).await.unwrap();
    handshake(&mut first).await;
    // Another viewer of the same stream doesn't need a new camera connection.
    let mut second = connect(&s, &s.admin, "?profile=grid", true).await.unwrap();
    handshake(&mut second).await;
    // A different profile would be a second connection: refused, with the
    // reason sent on the socket so the player can show it.
    let mut refused = connect(&s, &s.admin, "?profile=main", true).await.unwrap();
    match next(&mut refused).await {
        Message::Text(t) => {
            assert!(t.contains("\"failed\""), "{t}");
            assert!(t.contains("connection limit (1)"), "{t}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(s.app.state.streams.list().len(), 1);
}
