//! What a provider's upstream media source yields (ADR 0002): codec
//! parameters, then timestamped access units. Transport-independent: the AXIS
//! provider produces these from RTSP, the demo provider from bundled clips, and
//! the stream manager turns them into fMP4 for browsers.

use async_trait::async_trait;
use bytes::Bytes;

use crate::providers::{ProviderError, VideoCodec};

/// 90 kHz, the RTP/MPEG clock used for all media timestamps.
pub const TIMESCALE: u32 = 90_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoParams {
    /// H.264 in the MVP (H.265 is reported as unsupported for live view).
    pub codec: VideoCodec,
    /// RFC 6381 codec string for MSE, e.g. `avc1.4D401F`.
    pub rfc6381: String,
    pub width: u32,
    pub height: u32,
    /// A complete MP4 `avc1` sample entry box (with `avcC` inside).
    pub sample_entry: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFrame {
    /// Presentation time, 90 kHz.
    pub pts: i64,
    /// Decode time, 90 kHz. Equal to `pts` without B-frames.
    pub dts: i64,
    /// A random access point: decoding can start here.
    pub keyframe: bool,
    /// One access unit in AVC format (4-byte length-prefixed NAL units).
    pub data: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaEvent {
    /// Sent first, and again whenever the codec parameters change.
    Params(VideoParams),
    Frame(MediaFrame),
}

/// An open upstream media session. Dropping it closes the session.
#[async_trait]
pub trait MediaSource: Send {
    /// The next event; `Ok(None)` when the source ended cleanly.
    async fn next(&mut self) -> Result<Option<MediaEvent>, ProviderError>;
}
