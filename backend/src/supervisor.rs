//! Background service supervision (SPEC §10.4): restart a service with backoff if
//! it panics or exits early, and expose its health at `/api/v1/health`.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, Serialize, ToSchema, PartialEq, Eq)]
pub struct ServiceState {
    pub running: bool,
    pub restarts: u32,
}

/// Shared registry of supervised services.
#[derive(Debug, Clone, Default)]
pub struct ServiceHealth(Arc<Mutex<BTreeMap<&'static str, ServiceState>>>);

impl ServiceHealth {
    fn update(&self, name: &'static str, f: impl FnOnce(&mut ServiceState)) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(map.entry(name).or_default());
    }

    pub fn snapshot(&self) -> BTreeMap<String, ServiceState> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }
}

const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Run `make(cancel)` until `cancel` fires, restarting it with exponential backoff
/// (1 s doubling to 60 s) whenever it panics or returns early.
pub async fn supervise<F, Fut>(
    name: &'static str,
    health: ServiceHealth,
    cancel: CancellationToken,
    make: F,
) where
    F: Fn(CancellationToken) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut backoff = Duration::from_secs(1);
    loop {
        health.update(name, |s| s.running = true);
        let result = tokio::spawn(make(cancel.child_token())).await;
        health.update(name, |s| s.running = false);
        if cancel.is_cancelled() {
            return;
        }
        match result {
            Err(e) if e.is_panic() => {
                tracing::error!(service = name, "service panicked; restarting")
            }
            _ => tracing::error!(service = name, "service exited unexpectedly; restarting"),
        }
        health.update(name, |s| s.restarts = s.restarts.saturating_add(1));
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test(start_paused = true)]
    async fn restarts_after_panic_and_stops_on_cancel() {
        let health = ServiceHealth::default();
        let cancel = CancellationToken::new();
        let runs = Arc::new(AtomicU32::new(0));
        let task = tokio::spawn(supervise("flaky", health.clone(), cancel.clone(), {
            let runs = runs.clone();
            move |cancel: CancellationToken| {
                let n = runs.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        panic!("boom");
                    }
                    cancel.cancelled().await;
                }
            }
        }));
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        let state = health.snapshot()["flaky"].clone();
        assert_eq!(
            state,
            ServiceState {
                running: true,
                restarts: 2
            }
        );
        cancel.cancel();
        task.await.unwrap();
        assert!(!health.snapshot()["flaky"].running);
    }
}
