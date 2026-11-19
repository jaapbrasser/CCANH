//! Bandwidth limits and priorities for camera connections (SPEC §7.6,
//! Phase 9). Both limits are off by default (0 = no limit).
//!
//! - `max_upstreams`: at most this many camera connections at once.
//!   `detail_reserve` of them can only be used by detail views, so a full wall
//!   of grid tiles never stops someone opening a camera.
//! - `max_inbound_mbps`: no new connections while inbound video from cameras
//!   is above this rate (measured, so it reacts within a second or two).
//!
//! Joining a stream that's already running never counts against a limit.

use super::manager::{Priority, StreamInfo, StreamKey, StreamPolicy, StreamState};
use crate::settings::SettingsHandle;

pub struct LimitsPolicy {
    settings: SettingsHandle,
}

impl LimitsPolicy {
    pub fn new(settings: SettingsHandle) -> Self {
        Self { settings }
    }
}

impl StreamPolicy for LimitsPolicy {
    fn admit(
        &self,
        _key: &StreamKey,
        priority: Priority,
        active: &[StreamInfo],
    ) -> Result<(), String> {
        let s = self.settings.current().streaming;
        // Failed or stopped hubs no longer hold a camera connection.
        let live: Vec<&StreamInfo> = active
            .iter()
            .filter(|i| !matches!(i.state, StreamState::Failed | StreamState::Stopped))
            .collect();
        if s.max_upstreams > 0 {
            let limit = match priority {
                Priority::Detail => s.max_upstreams,
                Priority::Grid => s.max_upstreams.saturating_sub(s.detail_reserve),
            };
            if u32::try_from(live.len()).unwrap_or(u32::MAX) >= limit {
                return Err(match priority {
                    Priority::Grid if limit < s.max_upstreams => format!(
                        "The camera connection limit for grid tiles ({limit} of {}) is reached; the rest are kept for detail views.",
                        s.max_upstreams
                    ),
                    _ => format!("The camera connection limit ({}) is reached.", s.max_upstreams),
                });
            }
        }
        if s.max_inbound_mbps > 0 {
            let bits: u64 = live.iter().map(|i| i.in_bytes_per_sec * 8).sum();
            if bits >= u64::from(s.max_inbound_mbps) * 1_000_000 {
                return Err(format!(
                    "Video from cameras is at the {} Mbit/s limit.",
                    s.max_inbound_mbps
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::providers::StreamProfileId;
    use crate::settings::RuntimeSettings;
    use uuid::Uuid;

    fn policy(max: u32, reserve: u32, mbps: u32) -> LimitsPolicy {
        let mut s = RuntimeSettings::from_config(&Config::default());
        s.streaming.max_upstreams = max;
        s.streaming.detail_reserve = reserve;
        s.streaming.max_inbound_mbps = mbps;
        LimitsPolicy::new(SettingsHandle::new(s))
    }

    fn key() -> StreamKey {
        StreamKey {
            camera_id: Uuid::nil(),
            profile: StreamProfileId("main".into()),
        }
    }

    fn info(state: StreamState, bytes_per_sec: u64) -> StreamInfo {
        StreamInfo {
            camera_id: Uuid::now_v7(),
            profile: StreamProfileId("main".into()),
            state,
            subscribers: 1,
            codec: None,
            width: None,
            height: None,
            started_at: time::OffsetDateTime::now_utc(),
            startup_ms: None,
            bytes_in_total: 0,
            bytes_out_total: 0,
            in_bytes_per_sec: bytes_per_sec,
            out_bytes_per_sec: 0,
            reconnects: 0,
            errors: 0,
            lag_resyncs: 0,
            buffer_bytes: 0,
            last_error: None,
        }
    }

    #[test]
    fn unlimited_by_default() {
        let p = policy(0, 2, 0);
        let many = vec![info(StreamState::Running, 10_000_000); 500];
        assert!(p.admit(&key(), Priority::Grid, &many).is_ok());
    }

    #[test]
    fn grid_tiles_leave_room_for_detail_views() {
        let p = policy(4, 1, 0);
        let three = vec![info(StreamState::Running, 0); 3];
        assert!(
            p.admit(&key(), Priority::Grid, &three).is_err(),
            "3 of 4, one reserved"
        );
        assert!(p.admit(&key(), Priority::Detail, &three).is_ok());
        let four = vec![info(StreamState::Running, 0); 4];
        assert!(p.admit(&key(), Priority::Detail, &four).is_err());
        // Failed streams don't hold a connection.
        let mut mixed = three.clone();
        mixed[0].state = StreamState::Failed;
        assert!(p.admit(&key(), Priority::Grid, &mixed).is_ok());
    }

    #[test]
    fn inbound_bandwidth_cap() {
        let p = policy(0, 0, 10);
        let under = vec![info(StreamState::Running, 1_000_000)]; // 8 Mbit/s
        assert!(p.admit(&key(), Priority::Detail, &under).is_ok());
        let over = vec![info(StreamState::Running, 1_000_000); 2]; // 16 Mbit/s
        let err = p.admit(&key(), Priority::Detail, &over).unwrap_err();
        assert!(err.contains("10 Mbit/s"), "{err}");
    }
}
