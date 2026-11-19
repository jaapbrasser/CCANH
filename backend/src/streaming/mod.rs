//! Live video (SPEC §7, ADR 0002): one upstream connection per (camera,
//! profile) shared by every viewer, remuxed to fragmented MP4 for Media Source
//! Extensions. Buffering is transient and in memory only: nothing is ever
//! written to disk (this is not recording).

pub mod demo;
pub mod fmp4;
pub mod h264;
pub mod manager;
pub mod media;
pub mod opener;
pub mod policy;
pub mod profiles;
pub mod rtsp;

pub use manager::{Packet, StreamInfo, StreamKey, StreamManager, StreamState};
