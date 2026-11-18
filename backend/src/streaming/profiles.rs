//! Choosing stream profiles (SPEC §7.4): grid tiles use a low-resolution
//! profile, the detail view the main one. Per-camera overrides arrive in 5b.

use serde::Deserialize;
use utoipa::ToSchema;

use crate::providers::{StreamProfile, StreamProfileId, VideoCodec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ViewPurpose {
    Grid,
    #[default]
    Detail,
}

/// Profiles a browser can play: H.264, or unknown codec (camera default, which
/// is H.264 on AXIS unless configured otherwise).
pub fn playable(p: &StreamProfile) -> bool {
    matches!(p.codec, None | Some(VideoCodec::H264))
}

fn area(p: &StreamProfile) -> Option<u64> {
    Some(u64::from(p.width?) * u64::from(p.height?))
}

/// Pick a profile for `purpose`: the largest known resolution for detail, the
/// smallest for grid. Falls back to the first playable profile.
pub fn default_profile(
    profiles: &[StreamProfile],
    purpose: ViewPurpose,
) -> Option<StreamProfileId> {
    let candidates: Vec<&StreamProfile> = profiles.iter().filter(|p| playable(p)).collect();
    let with_area = candidates.iter().filter_map(|p| area(p).map(|a| (a, *p)));
    let chosen = match purpose {
        ViewPurpose::Detail => with_area.max_by_key(|(a, _)| *a).map(|(_, p)| p),
        ViewPurpose::Grid => with_area.min_by_key(|(a, _)| *a).map(|(_, p)| p),
    };
    chosen
        .or_else(|| candidates.first().copied())
        .map(|p| p.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, codec: Option<VideoCodec>, size: Option<(u32, u32)>) -> StreamProfile {
        StreamProfile {
            id: StreamProfileId(id.into()),
            name: id.into(),
            codec,
            width: size.map(|s| s.0),
            height: size.map(|s| s.1),
            fps: None,
            bitrate_kbps: None,
        }
    }

    #[test]
    fn picks_largest_for_detail_and_smallest_for_grid_ignoring_h265() {
        let profiles = [
            p("main", Some(VideoCodec::H264), Some((1920, 1080))),
            p("main-h265", Some(VideoCodec::H265), Some((3840, 2160))),
            p("grid", Some(VideoCodec::H264), Some((640, 360))),
            p("unknown", None, None),
        ];
        assert_eq!(
            default_profile(&profiles, ViewPurpose::Detail).unwrap().0,
            "main"
        );
        assert_eq!(
            default_profile(&profiles, ViewPurpose::Grid).unwrap().0,
            "grid"
        );
    }

    #[test]
    fn falls_back_to_first_playable() {
        let profiles = [
            p("h265", Some(VideoCodec::H265), Some((1, 1))),
            p("", None, None),
        ];
        assert_eq!(
            default_profile(&profiles, ViewPurpose::Detail).unwrap().0,
            ""
        );
        assert!(default_profile(&[], ViewPurpose::Grid).is_none());
    }
}
