//! Provider contract suite (SPEC §15.2): behaviour every [`CameraProvider`] must
//! have, independent of how it talks to cameras. Runs against DemoProvider now
//! and against AxisProvider-with-wiremock in Phase 4.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::{CameraProvider, ErrorCategory, StreamProfileId};

/// Upper bound for any single provider call in the suite: no unbounded waits.
const CALL_BOUND: Duration = Duration::from_secs(30);

/// Providers for the situations the contract covers.
pub struct Subjects {
    /// A reachable camera with valid credentials.
    pub healthy: Arc<dyn CameraProvider>,
    /// A camera that doesn't answer.
    pub unreachable: Arc<dyn CameraProvider>,
    /// A camera that rejects the credentials.
    pub auth_failure: Arc<dyn CameraProvider>,
}

async fn bounded<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(CALL_BOUND, fut)
        .await
        .unwrap_or_else(|_| panic!("{what} did not complete within {CALL_BOUND:?}"))
}

pub async fn run(s: Subjects) {
    identity(&*s.healthy).await;
    status(&*s.healthy).await;
    capabilities(&*s.healthy).await;
    streams(&*s.healthy).await;
    unreachable(&*s.unreachable).await;
    auth_failure(&*s.auth_failure).await;
    // Last: a restarted camera may stop answering for a while.
    restart(&s).await;
}

/// Restart is accepted by a healthy camera and reports the usual categories
/// for one that can't be reached or rejects the credentials.
async fn restart(s: &Subjects) {
    let err = bounded("restart (unreachable)", s.unreachable.restart())
        .await
        .expect_err("an unreachable camera can't be restarted");
    assert!(
        matches!(
            err.category(),
            ErrorCategory::Unreachable | ErrorCategory::Timeout
        ),
        "{err}"
    );
    let err = bounded("restart (auth failure)", s.auth_failure.restart())
        .await
        .expect_err("restart needs valid credentials");
    assert_eq!(err.category(), ErrorCategory::AuthFailed, "{err}");
    bounded("restart", s.healthy.restart())
        .await
        .expect("a healthy camera must accept a restart");
}

async fn identity(p: &dyn CameraProvider) {
    let probe = bounded("probe", p.probe()).await.expect("probe failed");
    assert!(
        probe.response_time_ms > 0,
        "probe must report a response time"
    );
    assert!(
        probe.device.manufacturer.is_some(),
        "probe must identify the manufacturer"
    );
    assert!(
        probe.device.model.is_some(),
        "probe must identify the model"
    );

    let info = bounded("get_device_info", p.get_device_info())
        .await
        .expect("get_device_info failed");
    // get_device_info may add detail (e.g. network info), but must agree on identity.
    assert_eq!(info.manufacturer, probe.device.manufacturer);
    assert_eq!(info.model, probe.device.model);
    assert_eq!(info.firmware_version, probe.device.firmware_version);
    assert_eq!(info.serial_number, probe.device.serial_number);
}

async fn status(p: &dyn CameraProvider) {
    let status = bounded("get_status", p.get_status())
        .await
        .expect("get_status failed");
    assert!(status.response_time_ms > 0);
}

async fn capabilities(p: &dyn CameraProvider) {
    let caps = bounded("get_capabilities", p.get_capabilities())
        .await
        .expect("get_capabilities failed");
    assert!(!caps.items.is_empty(), "capabilities must not be empty");
    let keys: HashSet<_> = caps.items.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(
        keys.len(),
        caps.items.len(),
        "capability keys must be unique"
    );
    for c in &caps.items {
        assert!(!c.label.is_empty(), "capability {} has no label", c.key);
        if !c.supported {
            assert!(
                c.version.is_none(),
                "unsupported {} must not report a version",
                c.key
            );
        }
    }
}

async fn streams(p: &dyn CameraProvider) {
    let profiles = bounded("get_stream_info", p.get_stream_info())
        .await
        .expect("get_stream_info failed");
    assert!(!profiles.is_empty(), "at least one stream profile");
    let ids: HashSet<_> = profiles.iter().map(|sp| sp.id.clone()).collect();
    assert_eq!(
        ids.len(),
        profiles.len(),
        "stream profile ids must be unique"
    );
    for sp in &profiles {
        assert!(!sp.name.is_empty(), "profile {:?} has no name", sp.id);
        if let (Some(w), Some(h)) = (sp.width, sp.height) {
            assert!(w > 0 && h > 0, "{} has a zero resolution", sp.id);
        }
    }

    // Opening a stream either works or is cleanly unsupported; it never panics,
    // hangs or reports a transport error for a healthy camera.
    let first = profiles[0].id.clone();
    match bounded(
        "open_stream",
        p.open_stream(&first, CancellationToken::new()),
    )
    .await
    {
        Ok(source) => drop(source),
        Err(e) => assert_eq!(e.category(), ErrorCategory::Unsupported, "{e}"),
    }
    let missing = StreamProfileId("no-such-profile".into());
    assert!(
        bounded(
            "open_stream(missing)",
            p.open_stream(&missing, CancellationToken::new())
        )
        .await
        .is_err(),
        "opening an unknown profile must fail"
    );
}

async fn unreachable(p: &dyn CameraProvider) {
    let reachability = [ErrorCategory::Unreachable, ErrorCategory::Timeout];
    let check = |what: &str, cat: ErrorCategory| {
        assert!(
            reachability.contains(&cat),
            "{what} on an unreachable camera gave {cat:?}"
        );
    };
    check(
        "probe",
        bounded("probe", p.probe()).await.unwrap_err().category(),
    );
    check(
        "get_status",
        bounded("get_status", p.get_status())
            .await
            .unwrap_err()
            .category(),
    );
    check(
        "get_device_info",
        bounded("get_device_info", p.get_device_info())
            .await
            .unwrap_err()
            .category(),
    );
    check(
        "get_capabilities",
        bounded("get_capabilities", p.get_capabilities())
            .await
            .unwrap_err()
            .category(),
    );
}

/// Wrong credentials: anonymous calls may still succeed, but an authenticated
/// call must report `AuthFailed` (never a transport error or a panic).
async fn auth_failure(p: &dyn CameraProvider) {
    if let Err(e) = bounded("probe", p.probe()).await {
        assert_eq!(e.category(), ErrorCategory::AuthFailed, "{e}");
    }
    let err = bounded("get_stream_info", p.get_stream_info())
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::AuthFailed, "{err}");
    assert!(
        !err.category().is_retryable(),
        "auth failures must not be retried in a loop"
    );
}

#[cfg(test)]
mod demo {
    use super::*;
    use crate::providers::demo::catalog::{self, Scenario};
    use crate::providers::demo::models::ALL_MODELS;
    use crate::providers::demo::DemoProvider;

    #[tokio::test(start_paused = true)]
    async fn demo_provider_passes_contract_for_every_model() {
        for (i, model) in ALL_MODELS.into_iter().enumerate() {
            // A host of its own per model: restart takes a demo host offline.
            let host = format!("203.0.113.{}", 10 + i);
            let make = |scenario| -> Arc<dyn CameraProvider> {
                Arc::new(DemoProvider::new(&host, None, model, scenario, 42))
            };
            run(Subjects {
                healthy: make(Scenario::Healthy),
                unreachable: make(Scenario::Offline),
                auth_failure: make(Scenario::AuthFailure),
            })
            .await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn every_healthy_fleet_camera_passes_identity_and_capabilities() {
        for cam in catalog::FLEET
            .iter()
            .filter(|c| c.scenario == Scenario::Healthy)
        {
            let p = DemoProvider::new(cam.host, Some(cam.name), cam.model, cam.scenario, 7);
            identity(&p).await;
            capabilities(&p).await;
        }
    }
}
