//! Simulated AXIS models with deliberately different capability sets, so demo
//! mode exercises the "Unsupported" paths (SPEC §12.2).

use crate::providers::{Capability, CapabilityGroup, StreamProfile, StreamProfileId, VideoCodec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKey {
    BoxQ1656,
    DomeP3265,
    PtzQ6135,
    MiniDomeM3086,
    BulletP1468,
    ModularF2105,
}

pub const ALL_MODELS: [ModelKey; 6] = [
    ModelKey::BoxQ1656,
    ModelKey::DomeP3265,
    ModelKey::PtzQ6135,
    ModelKey::MiniDomeM3086,
    ModelKey::BulletP1468,
    ModelKey::ModularF2105,
];

pub struct ModelProfile {
    pub model: &'static str,
    pub firmware: &'static str,
    /// Supported capability keys. Everything else in [`CATALOG`] is reported as
    /// unsupported.
    pub supported: &'static [&'static str],
    pub profiles: &'static [(&'static str, &'static str, VideoCodec, u32, u32, u32, u32)],
}

/// (key, group, label, version)
const CATALOG: &[(&str, CapabilityGroup, &str, Option<&str>)] = &[
    (
        "api.basic-device-info",
        CapabilityGroup::Device,
        "Basic device information",
        Some("1.3"),
    ),
    (
        "api.api-discovery",
        CapabilityGroup::Device,
        "API discovery",
        Some("1.0"),
    ),
    (
        "api.time",
        CapabilityGroup::Device,
        "Time settings",
        Some("1.1"),
    ),
    (
        "api.network-settings",
        CapabilityGroup::Network,
        "Network settings",
        Some("1.27"),
    ),
    ("network.https", CapabilityGroup::Network, "HTTPS", None),
    (
        "api.stream-profiles",
        CapabilityGroup::Video,
        "Stream profiles",
        Some("1.0"),
    ),
    ("video.h264", CapabilityGroup::Video, "H.264", None),
    ("video.h265", CapabilityGroup::Video, "H.265", None),
    ("video.mjpeg", CapabilityGroup::Video, "Motion JPEG", None),
    ("audio.input", CapabilityGroup::Audio, "Audio input", None),
    ("audio.output", CapabilityGroup::Audio, "Audio output", None),
    (
        "ptz.control",
        CapabilityGroup::Ptz,
        "Pan/tilt/zoom control",
        Some("3.0"),
    ),
    ("io.ports", CapabilityGroup::Io, "I/O ports", Some("1.1")),
    (
        "events.event-service",
        CapabilityGroup::Events,
        "Event service",
        Some("1.0"),
    ),
    ("storage.sd-card", CapabilityGroup::Storage, "SD card", None),
    (
        "analytics.motion",
        CapabilityGroup::Analytics,
        "Video motion detection",
        Some("4.5"),
    ),
    (
        "analytics.object",
        CapabilityGroup::Analytics,
        "Object analytics",
        Some("1.4"),
    ),
];

const MAIN_GRID_1080: &[(&str, &str, VideoCodec, u32, u32, u32, u32)] = &[
    (
        "main",
        "Main (1080p)",
        VideoCodec::H264,
        1920,
        1080,
        30,
        4000,
    ),
    ("grid", "Grid (360p)", VideoCodec::H264, 640, 360, 15, 512),
];

impl ModelKey {
    pub fn profile(self) -> ModelProfile {
        match self {
            ModelKey::BoxQ1656 => ModelProfile {
                model: "AXIS Q1656",
                firmware: "11.11.73",
                supported: &[
                    "api.basic-device-info",
                    "api.api-discovery",
                    "api.time",
                    "api.network-settings",
                    "network.https",
                    "api.stream-profiles",
                    "video.h264",
                    "video.h265",
                    "video.mjpeg",
                    "audio.input",
                    "audio.output",
                    "io.ports",
                    "events.event-service",
                    "storage.sd-card",
                    "analytics.motion",
                    "analytics.object",
                ],
                profiles: &[
                    ("main", "Main (4K)", VideoCodec::H264, 3840, 2160, 25, 8000),
                    (
                        "main-h265",
                        "Main (4K, H.265)",
                        VideoCodec::H265,
                        3840,
                        2160,
                        25,
                        5000,
                    ),
                    ("grid", "Grid (360p)", VideoCodec::H264, 640, 360, 15, 512),
                ],
            },
            ModelKey::DomeP3265 => ModelProfile {
                model: "AXIS P3265-LVE",
                firmware: "11.9.60",
                supported: &[
                    "api.basic-device-info",
                    "api.api-discovery",
                    "api.time",
                    "api.network-settings",
                    "network.https",
                    "api.stream-profiles",
                    "video.h264",
                    "video.h265",
                    "video.mjpeg",
                    "audio.input",
                    "io.ports",
                    "events.event-service",
                    "storage.sd-card",
                    "analytics.motion",
                    "analytics.object",
                ],
                profiles: MAIN_GRID_1080,
            },
            ModelKey::PtzQ6135 => ModelProfile {
                model: "AXIS Q6135-LE",
                firmware: "10.12.220",
                supported: &[
                    "api.basic-device-info",
                    "api.api-discovery",
                    "api.time",
                    "api.network-settings",
                    "network.https",
                    "api.stream-profiles",
                    "video.h264",
                    "video.h265",
                    "video.mjpeg",
                    "ptz.control",
                    "io.ports",
                    "events.event-service",
                    "storage.sd-card",
                    "analytics.motion",
                ],
                profiles: MAIN_GRID_1080,
            },
            ModelKey::MiniDomeM3086 => ModelProfile {
                model: "AXIS M3086-V",
                firmware: "11.11.73",
                supported: &[
                    "api.basic-device-info",
                    "api.api-discovery",
                    "api.time",
                    "api.network-settings",
                    "network.https",
                    "api.stream-profiles",
                    "video.h264",
                    "video.mjpeg",
                    "events.event-service",
                    "analytics.motion",
                ],
                profiles: &[
                    (
                        "main",
                        "Main (1440p)",
                        VideoCodec::H264,
                        2560,
                        1440,
                        30,
                        5000,
                    ),
                    ("grid", "Grid (360p)", VideoCodec::H264, 640, 360, 15, 512),
                ],
            },
            ModelKey::BulletP1468 => ModelProfile {
                model: "AXIS P1468-LE",
                firmware: "11.8.61",
                supported: &[
                    "api.basic-device-info",
                    "api.api-discovery",
                    "api.time",
                    "api.network-settings",
                    "network.https",
                    "api.stream-profiles",
                    "video.h264",
                    "video.h265",
                    "video.mjpeg",
                    "audio.input",
                    "audio.output",
                    "io.ports",
                    "events.event-service",
                    "storage.sd-card",
                    "analytics.motion",
                    "analytics.object",
                ],
                profiles: &[
                    ("main", "Main (4K)", VideoCodec::H264, 3840, 2160, 30, 8000),
                    ("grid", "Grid (360p)", VideoCodec::H264, 640, 360, 15, 512),
                ],
            },
            // An older, limited device: no stream-profiles API, no HTTPS, no H.265.
            ModelKey::ModularF2105 => ModelProfile {
                model: "AXIS F2105-RE",
                firmware: "9.80.3",
                supported: &[
                    "api.basic-device-info",
                    "api.time",
                    "api.network-settings",
                    "video.h264",
                    "video.mjpeg",
                    "events.event-service",
                ],
                profiles: &[(
                    "main",
                    "Main (1080p)",
                    VideoCodec::H264,
                    1920,
                    1080,
                    25,
                    3000,
                )],
            },
        }
    }
}

impl ModelProfile {
    pub fn capabilities(&self) -> Vec<Capability> {
        CATALOG
            .iter()
            .map(|(key, group, label, version)| {
                let supported = self.supported.contains(key);
                Capability {
                    key: (*key).to_owned(),
                    group: *group,
                    label: (*label).to_owned(),
                    supported,
                    version: if supported {
                        version.map(str::to_owned)
                    } else {
                        None
                    },
                    detail: (!supported).then(|| "Not available on this model".to_owned()),
                }
            })
            .collect()
    }

    pub fn stream_profiles(&self) -> Vec<StreamProfile> {
        self.profiles
            .iter()
            .map(|(id, name, codec, w, h, fps, kbps)| StreamProfile {
                id: StreamProfileId((*id).to_owned()),
                name: (*name).to_owned(),
                codec: Some(*codec),
                width: Some(*w),
                height: Some(*h),
                fps: Some(*fps),
                bitrate_kbps: Some(*kbps),
            })
            .collect()
    }

    pub fn supports(&self, key: &str) -> bool {
        self.supported.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_key_exists_in_catalog() {
        for model in ALL_MODELS {
            let profile = model.profile();
            for key in profile.supported {
                assert!(
                    CATALOG.iter().any(|(k, ..)| k == key),
                    "{} lists unknown capability {key}",
                    profile.model
                );
            }
        }
    }

    #[test]
    fn models_differ_and_include_unsupported_features() {
        for model in ALL_MODELS {
            let caps = model.profile().capabilities();
            assert!(
                caps.iter().any(|c| !c.supported),
                "{model:?} supports everything"
            );
            assert!(caps.iter().any(|c| c.supported));
        }
        let ptz: Vec<_> = ALL_MODELS
            .iter()
            .filter(|m| m.profile().supports("ptz.control"))
            .collect();
        assert_eq!(ptz.len(), 1, "exactly one PTZ model");
    }
}
