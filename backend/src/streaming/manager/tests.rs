//! Stream manager tests (SPEC §15.3): one upstream for N viewers, grace period,
//! slow-viewer isolation with keyframe resync, buffer caps, reconnect.

use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

use super::*;
use crate::config::Config;
use crate::providers::VideoCodec;
use crate::settings::RuntimeSettings;
use crate::streaming::media::MediaFrame;

/// Frames per GOP produced by the fake source.
const GOP: u64 = 5;
const FRAME_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Clone)]
enum Behaviour {
    /// Stream forever with frames of `size` bytes.
    Stream { size: usize },
    /// Stream `frames` frames, then fail with `error`.
    FailAfter { frames: u64, error: ProviderError },
    /// Refuse to open.
    Refuse(ProviderError),
}

struct FakeOpener {
    opens: AtomicUsize,
    plan: Mutex<VecDeque<Behaviour>>,
    default: Behaviour,
    open_delay: Duration,
}

impl FakeOpener {
    fn new(default: Behaviour) -> Arc<Self> {
        Arc::new(Self {
            opens: AtomicUsize::new(0),
            plan: Mutex::new(VecDeque::new()),
            default,
            open_delay: Duration::from_millis(20),
        })
    }

    fn with_plan(plan: Vec<Behaviour>, default: Behaviour) -> Arc<Self> {
        let o = Self::new(default);
        lock(&o.plan).extend(plan);
        o
    }

    fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }
}

struct FakeSource {
    behaviour: Behaviour,
    sent_params: bool,
    n: u64,
    cancel: CancellationToken,
}

#[async_trait]
impl MediaSource for FakeSource {
    async fn next(&mut self) -> Result<Option<MediaEvent>, ProviderError> {
        if !self.sent_params {
            self.sent_params = true;
            return Ok(Some(MediaEvent::Params(VideoParams {
                codec: VideoCodec::H264,
                rfc6381: "avc1.4D401F".into(),
                width: 640,
                height: 360,
                sample_entry: Bytes::from_static(b"\0\0\0\x08avc1"),
            })));
        }
        tokio::select! {
            () = self.cancel.cancelled() => return Ok(None),
            () = tokio::time::sleep(FRAME_INTERVAL) => {}
        }
        let size = match &self.behaviour {
            Behaviour::Stream { size } => *size,
            Behaviour::FailAfter { frames, error } => {
                if self.n >= *frames {
                    return Err(error.clone());
                }
                64
            }
            Behaviour::Refuse(_) => unreachable!(),
        };
        let i = self.n;
        self.n += 1;
        Ok(Some(MediaEvent::Frame(MediaFrame {
            pts: (i * 3600) as i64,
            dts: (i * 3600) as i64,
            keyframe: i.is_multiple_of(GOP),
            data: Bytes::from(vec![0u8; size]),
        })))
    }
}

#[async_trait]
impl SourceOpener for FakeOpener {
    async fn open(
        &self,
        _: &StreamKey,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.open_delay).await;
        let behaviour = lock(&self.plan)
            .pop_front()
            .unwrap_or_else(|| self.default.clone());
        if let Behaviour::Refuse(e) = behaviour {
            return Err(e);
        }
        Ok(Box::new(FakeSource {
            behaviour,
            sent_params: false,
            n: 0,
            cancel,
        }))
    }
}

fn settings(grace_secs: u64, stream_mb: u32, global_mb: u32) -> SettingsHandle {
    let mut s = RuntimeSettings::from_config(&Config::default());
    s.streaming.grace_period_secs = grace_secs;
    s.streaming.stream_buffer_mb = stream_mb;
    s.streaming.global_buffer_mb = global_mb;
    SettingsHandle::new(s)
}

fn manager(opener: Arc<FakeOpener>, grace_secs: u64) -> StreamManager {
    StreamManager::new(opener, settings(grace_secs, 4, 256), EventBus::default())
}

fn key(n: u128) -> StreamKey {
    StreamKey {
        camera_id: Uuid::from_u128(n),
        profile: StreamProfileId("main".into()),
    }
}

/// Next fragment, skipping state/init packets. `None` if the stream ended.
async fn next_fragment(sub: &mut Subscription) -> Option<Arc<Fragment>> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), sub.next())
            .await
            .ok()?
        {
            Some(Packet::Fragment(f)) => return Some(f),
            Some(_) => continue,
            None => return None,
        }
    }
}

async fn next_state(sub: &mut Subscription, wanted: StreamState) -> bool {
    loop {
        match tokio::time::timeout(Duration::from_secs(60), sub.next()).await {
            Ok(Some(Packet::State { state, .. })) if state == wanted => return true,
            Ok(Some(_)) => continue,
            _ => return false,
        }
    }
}

fn state_of(m: &StreamManager, k: &StreamKey) -> Option<StreamState> {
    m.list()
        .into_iter()
        .find(|s| s.camera_id == k.camera_id)
        .map(|s| s.state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn fifty_concurrent_subscribers_open_exactly_one_upstream() {
    let opener = FakeOpener::new(Behaviour::Stream { size: 64 });
    let m = manager(opener.clone(), 15);
    let barrier = Arc::new(tokio::sync::Barrier::new(50));
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let (m, barrier) = (m.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let mut sub = m.subscribe(key(1)).unwrap();
            // Every viewer's first fragment is a keyframe.
            let first = next_fragment(&mut sub).await.unwrap();
            assert!(first.keyframe);
            sub
        }));
    }
    let subs: Vec<_> = futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        opener.opens(),
        1,
        "N concurrent subscribers must share one open_stream"
    );
    let info = &m.list()[0];
    assert_eq!(info.subscribers, 50);
    assert_eq!(info.state, StreamState::Running);
    drop(subs);
}

#[tokio::test(start_paused = true)]
async fn late_viewer_starts_at_cached_keyframe_without_gaps() {
    let opener = FakeOpener::new(Behaviour::Stream { size: 64 });
    let m = manager(opener.clone(), 15);
    let mut a = m.subscribe(key(1)).unwrap();
    for _ in 0..(GOP + 2) {
        next_fragment(&mut a).await.unwrap(); // GOP cache now holds keyframe + 2
    }
    let mut b = m.subscribe(key(1)).unwrap();
    let mut got = Vec::new();
    for _ in 0..4 {
        got.push(next_fragment(&mut b).await.unwrap());
    }
    assert!(got[0].keyframe, "late viewer starts at the cached keyframe");
    assert!(
        got[1..].iter().all(|f| !f.keyframe),
        "then continuous frames, no skip"
    );
    assert_eq!(opener.opens(), 1);
    assert!(m.list()[0].startup_ms.is_some());
}

#[tokio::test(start_paused = true)]
async fn grace_period_keeps_upstream_and_resubscribe_cancels_shutdown() {
    let opener = FakeOpener::new(Behaviour::Stream { size: 64 });
    let m = manager(opener.clone(), 15);
    let k = key(1);
    let mut a = m.subscribe(k.clone()).unwrap();
    next_fragment(&mut a).await.unwrap();
    drop(a);
    assert_eq!(state_of(&m, &k), Some(StreamState::Draining));

    // Come back within the grace period: same upstream.
    tokio::time::sleep(Duration::from_secs(10)).await;
    let mut b = m.subscribe(k.clone()).unwrap();
    next_fragment(&mut b).await.unwrap();
    assert_eq!(opener.opens(), 1);
    assert_eq!(state_of(&m, &k), Some(StreamState::Running));

    // Leave again and let the grace period expire: upstream closes.
    drop(b);
    tokio::time::sleep(Duration::from_secs(16)).await;
    assert!(m.list().is_empty(), "stream closed after the grace period");
    assert_eq!(m.buffer_bytes(), 0, "buffers freed");

    // A new viewer opens a fresh upstream.
    let mut c = m.subscribe(k).unwrap();
    next_fragment(&mut c).await.unwrap();
    assert_eq!(opener.opens(), 2);
}

#[tokio::test(start_paused = true)]
async fn slow_viewer_does_not_stall_others_and_resyncs_at_keyframe() {
    let opener = FakeOpener::new(Behaviour::Stream { size: 64 });
    let m = manager(opener.clone(), 15);
    let mut fast = m.subscribe(key(1)).unwrap();
    let mut slow = m.subscribe(key(1)).unwrap(); // never reads for a while

    let total = SUBSCRIBER_QUEUE as u64 + 3 * GOP + 7;
    for i in 0..total {
        let f = next_fragment(&mut fast).await.unwrap();
        assert_eq!(
            f.keyframe,
            i % GOP == 0,
            "fast viewer sees every frame in order"
        );
    }

    // The slow viewer's queue filled while it wasn't reading. Draining it now:
    // a continuous run of frames, then (after the gap) a keyframe, never a
    // frame from the middle of a GOP.
    let mut slow_frames = Vec::new();
    while let Ok(Some(p)) = tokio::time::timeout(Duration::from_millis(1), slow.next()).await {
        if let Packet::Fragment(f) = p {
            slow_frames.push(f.keyframe);
        }
    }
    assert!(
        slow_frames.len() < total as usize,
        "slow viewer dropped frames"
    );
    let continuous = slow_frames
        .iter()
        .enumerate()
        .take_while(|(i, k)| **k == (*i as u64).is_multiple_of(GOP))
        .count();
    // The queue held state + init packets and then a continuous run of frames.
    assert!(
        continuous >= SUBSCRIBER_QUEUE - 3,
        "queue held a continuous run: continuous={continuous} len={}",
        slow_frames.len()
    );
    // The first frame delivered after the gap must be a keyframe.
    let resumed = if let Some(k) = slow_frames.get(continuous) {
        *k
    } else {
        next_fragment(&mut slow).await.unwrap().keyframe
    };
    assert!(resumed, "resync must start at a keyframe, never mid-GOP");
    assert!(m.list()[0].lag_resyncs >= 1);
}

#[tokio::test(start_paused = true)]
async fn buffer_caps_are_respected_per_stream_and_globally() {
    // 600 KiB frames, GOP of 5 → 3 MiB per GOP. Stream cap 2 MiB, global 4 MiB.
    let opener = FakeOpener::new(Behaviour::Stream { size: 600 * 1024 });
    let m = StreamManager::new(opener, settings(15, 2, 4), EventBus::default());
    let mut subs = Vec::new();
    for n in 0..4 {
        subs.push(m.subscribe(key(n)).unwrap());
    }
    let global_cap = 4 * 1024 * 1024;
    let stream_cap = 2 * 1024 * 1024;
    for _ in 0..(3 * GOP) {
        for s in &mut subs {
            next_fragment(s).await.unwrap();
        }
        assert!(
            m.buffer_bytes() <= global_cap,
            "global cap exceeded: {}",
            m.buffer_bytes()
        );
        for info in m.list() {
            assert!(
                info.buffer_bytes <= stream_cap,
                "stream cap exceeded: {}",
                info.buffer_bytes
            );
        }
    }
    let total: usize = m.list().iter().map(|i| i.buffer_bytes).sum();
    assert_eq!(
        total,
        m.buffer_bytes(),
        "global accounting matches the streams"
    );
}

#[tokio::test(start_paused = true)]
async fn reconnects_after_upstream_failure_and_tells_viewers() {
    let opener = FakeOpener::with_plan(
        vec![Behaviour::FailAfter {
            frames: 3,
            error: ProviderError::Unreachable("cable unplugged".into()),
        }],
        Behaviour::Stream { size: 64 },
    );
    let m = manager(opener.clone(), 15);
    let mut sub = m.subscribe(key(1)).unwrap();
    assert!(next_state(&mut sub, StreamState::Reconnecting).await);
    assert!(next_state(&mut sub, StreamState::Running).await);
    assert!(
        next_fragment(&mut sub).await.unwrap().keyframe,
        "after reconnecting, viewers resume at a keyframe"
    );
    assert_eq!(opener.opens(), 2);
    let info = &m.list()[0];
    assert_eq!(info.reconnects, 1);
    assert!(info.errors >= 1);
}

#[tokio::test(start_paused = true)]
async fn unsupported_and_auth_failures_fail_without_retrying() {
    for error in [
        ProviderError::unsupported("H.265"),
        ProviderError::AuthFailed("401".into()),
    ] {
        let opener = FakeOpener::new(Behaviour::Refuse(error.clone()));
        let m = manager(opener.clone(), 15);
        let mut sub = m.subscribe(key(1)).unwrap();
        assert!(next_state(&mut sub, StreamState::Failed).await, "{error}");
        // The stream then ends for the viewer and the hub is gone.
        while sub.next().await.is_some() {}
        assert!(m.list().is_empty());
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(opener.opens(), 1, "no retry loop on {error}");
    }
}

#[tokio::test(start_paused = true)]
async fn force_stop_disconnects_viewers() {
    let opener = FakeOpener::new(Behaviour::Stream { size: 64 });
    let m = manager(opener, 15);
    let mut sub = m.subscribe(key(1)).unwrap();
    next_fragment(&mut sub).await.unwrap();
    assert!(m.stop(&key(1)));
    assert!(next_state(&mut sub, StreamState::Stopped).await);
    assert!(sub.next().await.is_none());
    assert!(m.list().is_empty());
    assert!(!m.stop(&key(1)));
}

#[tokio::test(start_paused = true)]
async fn policy_can_refuse_new_streams() {
    struct One;
    impl StreamPolicy for One {
        fn admit(&self, _: &StreamKey, _: Priority, active: &[StreamInfo]) -> Result<(), String> {
            if active.is_empty() {
                Ok(())
            } else {
                Err("limit of 1 stream".into())
            }
        }
    }
    let opener = FakeOpener::new(Behaviour::Stream { size: 64 });
    let m = StreamManager::with_policy(
        opener,
        settings(15, 4, 256),
        EventBus::default(),
        Arc::new(One),
    );
    let _a = m.subscribe(key(1)).unwrap();
    let _a2 = m.subscribe(key(1)).unwrap(); // joining an existing stream is fine
    assert!(matches!(m.subscribe(key(2)), Err(StreamError::Refused(_))));
}
