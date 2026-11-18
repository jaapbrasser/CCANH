//! Load generator: N WebSocket viewers on one profile, plus optionally one slow
//! viewer that reads with a delay. Prints per-viewer throughput and time to the
//! first keyframe fragment.
//!
//!   cargo run --release --bin load -- <viewers> <seconds> [slow]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(10);
    let secs: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(20);
    let slow = args.next().is_some_and(|a| a == "slow");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "main".into());
    let url = format!("ws://127.0.0.1:9000/ws/{profile}");

    let total = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for i in 0..n + usize::from(slow) {
        let url = url.clone();
        let total = total.clone();
        let is_slow = slow && i == n;
        tasks.push(tokio::spawn(async move {
            let start = Instant::now();
            let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let mut first_media: Option<Duration> = None;
            let mut bytes = 0u64;
            let mut msgs = 0u64;
            let deadline = start + Duration::from_secs(secs);
            while Instant::now() < deadline {
                let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await else { break };
                if let Message::Binary(b) = msg {
                    msgs += 1;
                    bytes += b.len() as u64;
                    if msgs >= 2 && first_media.is_none() {
                        first_media = Some(start.elapsed());
                    }
                }
                if is_slow {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
            }
            total.fetch_add(bytes, Ordering::Relaxed);
            (is_slow, bytes, msgs, first_media)
        }));
    }
    let mut firsts = Vec::new();
    for t in tasks {
        let (is_slow, bytes, msgs, first) = t.await.unwrap();
        let kbps = bytes as f64 * 8.0 / 1000.0 / secs as f64;
        if is_slow {
            println!("slow viewer: {kbps:.0} kbit/s, {msgs} messages");
        } else if let Some(f) = first {
            firsts.push(f.as_millis());
        }
    }
    firsts.sort_unstable();
    let total = total.load(Ordering::Relaxed);
    println!(
        "viewers={n} total_out={:.1} Mbit/s per_viewer={:.0} kbit/s first_media_ms p50={} max={}",
        total as f64 * 8.0 / 1e6 / secs as f64,
        total as f64 * 8.0 / 1000.0 / secs as f64 / (n.max(1) as f64),
        firsts.get(firsts.len() / 2).copied().unwrap_or(0),
        firsts.last().copied().unwrap_or(0),
    );
}
