//! RTSP upstream (ADR 0002): one `retina` session per (camera, profile),
//! RTP over the RTSP TCP connection, H.264 only, no audio.
//!
//! Credentials are handed to retina, which answers the camera's Digest (or
//! Basic) challenge; they never appear in the URL, so URLs are safe to log.
//! Video passes through untouched: no decoding or transcoding.

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use retina::client::{
    Credentials as RtspCredentials, Demuxed, PlayOptions, Session, SessionOptions, SetupOptions,
    Transport,
};
use retina::codec::{CodecItem, FrameFormat, ParametersRef};
use secrecy::ExposeSecret;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::media::{MediaEvent, MediaFrame, MediaSource, VideoParams, TIMESCALE};
use crate::credentials::Credentials;
use crate::providers::{ProviderError, VideoCodec};

/// How long a running stream may go without a video frame before it counts as
/// dropped (cameras send several frames a second, even for static scenes).
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// What to open.
pub struct RtspRequest {
    /// `rtsp://host[:port]/path?query`, without credentials.
    pub url: Url,
    pub credentials: Option<Credentials>,
    /// Bounds each of DESCRIBE, SETUP and PLAY.
    pub timeout: Duration,
}

/// Open a session (DESCRIBE, SETUP, PLAY) and return it as a media source.
/// Errors here are what the stream manager classifies: auth failures and
/// unsupported streams are permanent, network trouble is retried.
pub async fn open(
    req: RtspRequest,
    cancel: CancellationToken,
) -> Result<Box<dyn MediaSource>, ProviderError> {
    debug_assert!(
        req.url.password().is_none(),
        "credentials never go in the URL"
    );
    let options = SessionOptions::default()
        .creds(req.credentials.map(|c| RtspCredentials {
            username: c.username,
            password: c.password.expose_secret().to_owned(),
        }))
        .user_agent(concat!("crane/", env!("CARGO_PKG_VERSION")).to_owned());

    let step = |what: &'static str| {
        move |e: retina::Error| classify(e.status_code(), &e.to_string(), what)
    };

    let mut session = bounded(
        &cancel,
        req.timeout,
        "DESCRIBE",
        Session::describe(req.url, options),
    )
    .await?
    .map_err(step("DESCRIBE"))?;
    let video = pick_video(
        session
            .streams()
            .iter()
            .map(|s| (s.media(), s.encoding_name())),
    )?;
    bounded(
        &cancel,
        req.timeout,
        "SETUP",
        session.setup(
            video,
            SetupOptions::default()
                .transport(Transport::default()) // TCP interleaved
                .frame_format(FrameFormat::MP4),
        ),
    )
    .await?
    .map_err(step("SETUP"))?;
    let playing = bounded(
        &cancel,
        req.timeout,
        "PLAY",
        session.play(PlayOptions::default()),
    )
    .await?
    .map_err(step("PLAY"))?;
    let demuxed = playing
        .demuxed()
        .map_err(|e| ProviderError::Protocol(first_line(&e.to_string())))?;
    Ok(Box::new(RtspMediaSource {
        demuxed,
        video,
        have_params: false,
        pending: None,
        cancel,
    }))
}

/// Run one RTSP step with a timeout, giving up early on cancellation.
async fn bounded<T>(
    cancel: &CancellationToken,
    timeout: Duration,
    what: &'static str,
    fut: impl std::future::Future<Output = T>,
) -> Result<T, ProviderError> {
    tokio::select! {
        () = cancel.cancelled() => Err(ProviderError::Unreachable("stream cancelled".into())),
        r = tokio::time::timeout(timeout, fut) => {
            r.map_err(|_| ProviderError::Timeout(format!("RTSP {what} took longer than {}s", timeout.as_secs())))
        }
    }
}

/// Choose the H.264 video stream from (media, encoding) pairs.
fn pick_video<'a>(
    streams: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<usize, ProviderError> {
    let streams: Vec<_> = streams.collect();
    if let Some(i) = streams
        .iter()
        .position(|(m, e)| *m == "video" && e.eq_ignore_ascii_case("h264"))
    {
        return Ok(i);
    }
    let video: Vec<&str> = streams
        .iter()
        .filter(|(m, _)| *m == "video")
        .map(|(_, e)| *e)
        .collect();
    Err(match video.first() {
        Some(e) if e.eq_ignore_ascii_case("h265") => {
            ProviderError::unsupported("H.265 live video (browsers here play H.264 only)")
        }
        Some(e) => ProviderError::unsupported(format!("{e} live video (only H.264 is supported)")),
        None => ProviderError::unsupported("live video (the camera offered no video stream)"),
    })
}

/// Map an RTSP failure while opening to a provider error category.
fn classify(status: Option<u16>, message: &str, step: &str) -> ProviderError {
    match status {
        Some(401) => ProviderError::AuthFailed("the camera rejected the RTSP credentials".into()),
        Some(403) => {
            ProviderError::AuthFailed("the camera account isn't allowed to view this stream".into())
        }
        // Unknown stream profile, bad parameters, codec the camera can't send.
        Some(code @ (400 | 404 | 406 | 415 | 451 | 457 | 461)) => {
            ProviderError::unsupported(format!("this stream (RTSP {step} returned {code})"))
        }
        // Too many sessions or not enough bandwidth on the camera: try again later.
        Some(code @ (453 | 503)) => {
            ProviderError::RateLimited(format!("the camera is busy (RTSP {code})"))
        }
        Some(code) => ProviderError::Protocol(format!("RTSP {step} returned {code}")),
        None => {
            let m = message.to_ascii_lowercase();
            let network = [
                "connect",
                "refused",
                "unreachable",
                "reset",
                "timed out",
                "eof",
                "closed",
                "broken pipe",
                "no route",
                "dns",
                "lookup",
            ];
            if network.iter().any(|w| m.contains(w)) {
                ProviderError::Unreachable(format!("RTSP {step}: {}", first_line(message)))
            } else {
                ProviderError::Protocol(format!("RTSP {step}: {}", first_line(message)))
            }
        }
    }
}

/// Retina's messages carry multi-line connection context; keep the summary.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    line.chars().take(200).collect()
}

/// Convert an RTP timestamp in `clock_rate` units to [`TIMESCALE`].
fn to_timescale(elapsed: i64, clock_rate: u32) -> i64 {
    if clock_rate == TIMESCALE {
        return elapsed;
    }
    let v = i128::from(elapsed) * i128::from(TIMESCALE) / i128::from(clock_rate.max(1));
    i64::try_from(v).unwrap_or(i64::MAX)
}

struct RtspMediaSource {
    demuxed: Demuxed,
    video: usize,
    have_params: bool,
    /// A frame held back while its new parameters are sent first.
    pending: Option<MediaFrame>,
    cancel: CancellationToken,
}

impl RtspMediaSource {
    fn params(&self) -> Result<VideoParams, ProviderError> {
        let Some(ParametersRef::Video(p)) = self.demuxed.streams()[self.video].parameters() else {
            return Err(ProviderError::Protocol(
                "the camera sent video before its parameters".into(),
            ));
        };
        let (width, height) = p.pixel_dimensions();
        let sample_entry = p
            .mp4_sample_entry()
            .build()
            .map_err(|e| ProviderError::Protocol(first_line(&e.to_string())))?;
        Ok(VideoParams {
            codec: VideoCodec::H264,
            rfc6381: p.rfc6381_codec().to_owned(),
            width,
            height,
            sample_entry: sample_entry.into(),
        })
    }
}

#[async_trait]
impl MediaSource for RtspMediaSource {
    async fn next(&mut self) -> Result<Option<MediaEvent>, ProviderError> {
        if let Some(frame) = self.pending.take() {
            return Ok(Some(MediaEvent::Frame(frame)));
        }
        loop {
            let item = tokio::select! {
                () = self.cancel.cancelled() => return Ok(None),
                r = tokio::time::timeout(IDLE_TIMEOUT, self.demuxed.next()) => r,
            };
            let item = match item {
                Err(_) => {
                    return Err(ProviderError::Timeout(format!(
                        "no video from the camera for {}s",
                        IDLE_TIMEOUT.as_secs()
                    )))
                }
                Ok(None) => return Ok(None),
                // Once playing, any failure is treated as a dropped connection
                // and retried by the stream manager.
                Ok(Some(Err(e))) => {
                    return Err(ProviderError::Unreachable(format!(
                        "RTSP stream interrupted: {}",
                        first_line(&e.to_string())
                    )))
                }
                Ok(Some(Ok(item))) => item,
            };
            let CodecItem::VideoFrame(frame) = item else {
                continue;
            };
            if frame.stream_id() != self.video {
                continue;
            }
            let ts = frame.timestamp();
            let t = to_timescale(ts.elapsed(), ts.clock_rate().get());
            let new_params = !self.have_params || frame.has_new_parameters();
            let media = MediaFrame {
                pts: t,
                dts: t,
                keyframe: frame.is_random_access_point(),
                data: Bytes::from(frame.into_data()),
            };
            if new_params {
                let params = self.params()?;
                self.have_params = true;
                self.pending = Some(media);
                return Ok(Some(MediaEvent::Params(params)));
            }
            return Ok(Some(MediaEvent::Frame(media)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ErrorCategory;

    #[test]
    fn picks_h264_and_explains_other_codecs() {
        assert_eq!(
            pick_video([("audio", "mpeg4-generic"), ("video", "H264")].into_iter()).unwrap(),
            1
        );
        let e = pick_video([("video", "h265")].into_iter()).unwrap_err();
        assert_eq!(e.category(), ErrorCategory::Unsupported);
        assert!(e.to_string().contains("H.265"), "{e}");
        assert_eq!(
            pick_video([("audio", "pcmu")].into_iter())
                .unwrap_err()
                .category(),
            ErrorCategory::Unsupported
        );
    }

    #[test]
    fn classifies_open_failures() {
        let c = |s, m| classify(s, m, "DESCRIBE").category();
        assert_eq!(c(Some(401), ""), ErrorCategory::AuthFailed);
        assert_eq!(c(Some(403), ""), ErrorCategory::AuthFailed);
        assert_eq!(c(Some(404), ""), ErrorCategory::Unsupported);
        assert_eq!(c(Some(503), ""), ErrorCategory::RateLimited);
        assert_eq!(c(Some(500), ""), ErrorCategory::ProtocolError);
        assert_eq!(
            c(
                None,
                "Unable to connect to RTSP server: Connection refused (os error 61)\n\nconn: x"
            ),
            ErrorCategory::Unreachable
        );
        assert_eq!(
            c(None, "RTSP framing error: bad header"),
            ErrorCategory::ProtocolError
        );
        // Only the first line of retina's message is kept.
        let e = classify(None, "Connection reset\nconn: 192.0.2.1:554", "PLAY");
        assert!(!e.to_string().contains("conn:"), "{e}");
    }

    #[test]
    fn converts_clock_rates() {
        assert_eq!(to_timescale(3600, TIMESCALE), 3600);
        assert_eq!(to_timescale(1000, 1000), 90_000);
    }
}
