//! RTSP source against a real RTSP server. Ignored by default: start the
//! local test source first (`pwsh scripts/rtsp-test-source.ps1`, needs ffmpeg
//! and mediamtx), then run
//! `cargo test -p crane --test rtsp_source -- --ignored`.
//! Override the URL with `CRANE_RTSP_TEST_URL`.

use std::time::Duration;

use crane::streaming::media::MediaEvent;
use crane::streaming::rtsp::{open, RtspRequest};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "needs a local RTSP source (scripts/rtsp-test-source.ps1)"]
async fn plays_from_a_real_rtsp_server() {
    let url = std::env::var("CRANE_RTSP_TEST_URL")
        .unwrap_or_else(|_| "rtsp://127.0.0.1:8554/test/grid".into());
    let mut src = open(
        RtspRequest {
            url: url.parse().unwrap(),
            credentials: None,
            timeout: Duration::from_secs(5),
        },
        CancellationToken::new(),
    )
    .await
    .expect("is the RTSP test source running?");

    let Some(MediaEvent::Params(p)) = src.next().await.unwrap() else {
        panic!("parameters come first")
    };
    assert!(p.rfc6381.starts_with("avc1."), "{}", p.rfc6381);
    assert!(p.width > 0 && p.height > 0);

    // A keyframe arrives within one GOP (2 s for the test source), and
    // timestamps increase.
    let mut last = i64::MIN;
    let mut saw_keyframe = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && !saw_keyframe {
        if let Some(MediaEvent::Frame(f)) = src.next().await.unwrap() {
            assert!(f.dts > last);
            last = f.dts;
            saw_keyframe |= f.keyframe;
        }
    }
    assert!(saw_keyframe);
}
