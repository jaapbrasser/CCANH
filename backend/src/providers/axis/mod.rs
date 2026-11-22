//! AXIS camera provider over VAPIX (SPEC §4).
//!
//! Every endpoint used here is logged in `docs/vapix.md` with the documentation
//! page it was checked against. Calls documented as anonymous are made without
//! credentials; everything else authenticates. Optional, higher-privilege data
//! (network settings need Administrator) is best-effort: a lower-privileged
//! account simply gets less detail, not an error.

mod client;
pub mod digest;
pub mod tls;

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use self::client::{Access, VapixClient};
use super::{
    CameraCapabilities, CameraProvider, CameraStatus, Capability, CapabilityGroup, DeviceInfo,
    ErrorCategory, MediaSource, NetworkInfo, ProbeResult, ProviderError, ProviderTarget,
    StreamProfile, StreamProfileId, VideoCodec,
};
use crate::credentials::Credentials;

pub const BASIC_DEVICE_INFO: &str = "/axis-cgi/basicdeviceinfo.cgi";
pub const API_DISCOVERY: &str = "/axis-cgi/apidiscovery.cgi";
pub const SYSTEM_READY: &str = "/axis-cgi/systemready.cgi";
pub const STREAM_PROFILES: &str = "/axis-cgi/streamprofile.cgi";
pub const NETWORK_SETTINGS: &str = "/axis-cgi/network_settings.cgi";
pub const FIRMWARE_MANAGEMENT: &str = "/axis-cgi/firmwaremanagement.cgi";
/// RTSP media path (VAPIX "Video streaming"); see `docs/vapix.md`.
pub const RTSP_MEDIA: &str = "/axis-media/media.amp";
/// Default RTSP server port on AXIS devices.
pub const RTSP_PORT: u16 = 554;

pub struct AxisProvider {
    client: Result<VapixClient, ProviderError>,
    host: String,
    rtsp_port: u16,
    credentials: Option<Credentials>,
    timeout: Duration,
}

impl AxisProvider {
    pub fn new(target: &ProviderTarget, timeout: Duration) -> Self {
        Self {
            client: VapixClient::new(target, timeout),
            host: target.host.clone(),
            rtsp_port: RTSP_PORT,
            credentials: target.credentials.clone(),
            timeout,
        }
    }

    /// Use a non-standard RTSP port (tests; cameras behind port forwarding).
    #[must_use]
    pub fn with_rtsp_port(mut self, port: u16) -> Self {
        self.rtsp_port = port;
        self
    }

    /// The RTSP URL for a stream profile (the empty id is the camera default).
    /// Never contains credentials.
    pub fn rtsp_url(&self, profile: &StreamProfileId) -> Result<url::Url, ProviderError> {
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host) // IPv6 literal
        } else {
            self.host.clone()
        };
        let mut url = url::Url::parse(&format!("rtsp://{host}:{}{RTSP_MEDIA}", self.rtsp_port))
            .map_err(|e| ProviderError::Internal(format!("RTSP URL: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("videocodec", "h264");
            q.append_pair("audio", "0");
            if !profile.0.is_empty() {
                q.append_pair("streamprofile", &profile.0);
            }
        }
        Ok(url)
    }

    fn client(&self) -> Result<&VapixClient, ProviderError> {
        self.client.as_ref().map_err(Clone::clone)
    }

    /// POST a JSON request and unwrap VAPIX's `{data}` / `{error}` envelope.
    async fn call(
        &self,
        path: &'static str,
        body: Value,
        access: Access,
    ) -> Result<(Value, u32), ProviderError> {
        let (value, elapsed) = self.client()?.post_json(path, body, access).await?;
        if let Some(err) = value.get("error") {
            return Err(envelope_error(path, err));
        }
        let data = value
            .get("data")
            .cloned()
            .ok_or_else(|| ProviderError::Protocol(format!("{path}: response has no data")))?;
        Ok((
            data,
            u32::try_from(elapsed.as_millis())
                .unwrap_or(u32::MAX)
                .max(1),
        ))
    }

    async fn unrestricted_info(&self) -> Result<(DeviceInfo, u32), ProviderError> {
        let (data, ms) = self
            .call(
                BASIC_DEVICE_INFO,
                json!({ "apiVersion": "1.0", "method": "getAllUnrestrictedProperties" }),
                Access::Anonymous,
            )
            .await?;
        Ok((device_from_properties(&data), ms))
    }

    async fn network_info(&self) -> Result<NetworkSummary, ProviderError> {
        let (data, _) = self
            .call(
                NETWORK_SETTINGS,
                json!({ "apiVersion": "1.0", "method": "getNetworkInfo" }),
                Access::Authenticated,
            )
            .await?;
        Ok(parse_network(&data))
    }
}

/// Map a VAPIX JSON error envelope. Codes from the API documentation:
/// 2000 unsupported version, 2005 unsupported method.
fn envelope_error(path: &str, err: &Value) -> ProviderError {
    let code = err.get("code").and_then(Value::as_i64).unwrap_or_default();
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("error")
        .chars()
        .take(200)
        .collect::<String>();
    match code {
        2000 | 2005 => ProviderError::unsupported(format!("{path} ({message})")),
        _ => ProviderError::Protocol(format!("{path}: {message} (code {code})")),
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Map Basic device information properties (`propertyList`).
fn device_from_properties(data: &Value) -> DeviceInfo {
    let p = data.get("propertyList").unwrap_or(&Value::Null);
    let brand = str_field(p, "Brand");
    let model = str_field(p, "ProdNbr").map(|nbr| match &brand {
        Some(b) if !nbr.starts_with(b.as_str()) => format!("{b} {nbr}"),
        _ => nbr,
    });
    DeviceInfo {
        manufacturer: brand,
        model,
        firmware_version: str_field(p, "Version"),
        serial_number: str_field(p, "SerialNumber"),
        mac_address: None,
        device_name: str_field(p, "ProdFullName"),
        hostname: None,
        network: None,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct NetworkSummary {
    hostname: Option<String>,
    mac: Option<String>,
    info: NetworkInfo,
}

/// Map `getNetworkInfo`: the first wired device with IPv4, else the first device.
fn parse_network(data: &Value) -> NetworkSummary {
    let hostname = data
        .pointer("/system/hostname/hostname")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let devices = data
        .get("devices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let device = devices
        .iter()
        .find(|d| d.get("type").and_then(Value::as_str) == Some("wired"))
        .or_else(|| devices.first());
    let Some(d) = device else {
        return NetworkSummary {
            hostname,
            ..Default::default()
        };
    };
    let ipv4 = d.get("IPv4").unwrap_or(&Value::Null);
    let addr = ipv4
        .get("addresses")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("scope").and_then(Value::as_str) == Some("global"))
                .or_else(|| a.first())
        });
    let prefix = addr
        .and_then(|a| a.get("prefixLength"))
        .and_then(Value::as_u64);
    NetworkSummary {
        hostname,
        mac: d
            .get("macAddress")
            .and_then(Value::as_str)
            .map(|m| m.to_ascii_uppercase()),
        info: NetworkInfo {
            ipv4_address: addr
                .and_then(|a| a.get("address"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            subnet_mask: prefix
                .and_then(|p| u8::try_from(p).ok())
                .and_then(prefix_to_mask),
            gateway: str_field(ipv4, "defaultRouter"),
            dhcp: ipv4
                .get("configurationMode")
                .and_then(Value::as_str)
                .map(|m| m == "dhcp"),
        },
    }
}

fn prefix_to_mask(prefix: u8) -> Option<String> {
    if prefix > 32 {
        return None;
    }
    let bits = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Some(std::net::Ipv4Addr::from(bits).to_string())
}

/// Group an API Discovery entry by its id. Unknown ids land in `Device`.
fn group_for(id: &str) -> CapabilityGroup {
    let id = id.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| id.contains(w));
    if has(&["ptz", "pan", "tilt", "zoom", "guard-tour"]) {
        CapabilityGroup::Ptz
    } else if has(&["audio", "speaker", "siren"]) {
        CapabilityGroup::Audio
    } else if has(&["io-", "port", "relay", "input", "output"]) {
        CapabilityGroup::Io
    } else if has(&["event", "mqtt", "action"]) {
        CapabilityGroup::Events
    } else if has(&["storage", "recording", "disk"]) {
        CapabilityGroup::Storage
    } else if has(&["analytics", "object", "motion", "metadata"]) {
        CapabilityGroup::Analytics
    } else if has(&[
        "stream",
        "video",
        "image",
        "overlay",
        "capture",
        "zipstream",
    ]) {
        CapabilityGroup::Video
    } else if has(&[
        "network",
        "ntp",
        "mdns",
        "dns",
        "ssh",
        "snmp",
        "firewall",
        "certificate",
    ]) {
        CapabilityGroup::Network
    } else {
        CapabilityGroup::Device
    }
}

/// APIs this application relies on; reported as unsupported when absent.
const RELIED_ON: &[(&str, CapabilityGroup, &str)] = &[
    (
        "basic-device-info",
        CapabilityGroup::Device,
        "Basic device information",
    ),
    ("api-discovery", CapabilityGroup::Device, "API discovery"),
    ("stream-profiles", CapabilityGroup::Video, "Stream profiles"),
];

fn capabilities_from_api_list(data: &Value) -> CameraCapabilities {
    let list = data
        .get("apiList")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Keep the highest released version per API id.
    let mut best: Vec<(String, String, String)> = Vec::new(); // (id, name, version)
    for api in &list {
        let (Some(id), Some(version)) = (str_field(api, "id"), str_field(api, "version")) else {
            continue;
        };
        if api.get("status").and_then(Value::as_str) == Some("deprecated") {
            continue;
        }
        let name = str_field(api, "name").unwrap_or_else(|| id.clone());
        match best.iter_mut().find(|(i, ..)| *i == id) {
            Some(entry) if version_gt(&version, &entry.2) => *entry = (id, name, version),
            Some(_) => {}
            None => best.push((id, name, version)),
        }
    }
    let mut items: Vec<Capability> = best
        .into_iter()
        .map(|(id, name, version)| Capability {
            key: format!("api.{id}"),
            group: group_for(&id),
            label: name,
            supported: true,
            version: Some(version),
            detail: None,
        })
        .collect();
    for (id, group, label) in RELIED_ON {
        let key = format!("api.{id}");
        if !items.iter().any(|c| c.key == key) {
            items.push(Capability {
                key,
                group: *group,
                label: (*label).to_owned(),
                supported: false,
                version: None,
                detail: Some("Not offered by this device's firmware".into()),
            });
        }
    }
    items.sort_by(|a, b| a.key.cmp(&b.key));
    CameraCapabilities { items }
}

fn version_gt(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.split('.').filter_map(|p| p.parse().ok()).collect() };
    parse(a) > parse(b)
}

/// Parse a stream profile's `parameters` string (`key=value&key=value`).
fn profile_from(name: &str, description: Option<String>, params: &str) -> StreamProfile {
    let mut p = StreamProfile {
        id: StreamProfileId(name.to_owned()),
        name: description
            .filter(|d| !d.is_empty())
            .map_or_else(|| name.to_owned(), |d| format!("{name} – {d}")),
        codec: None,
        width: None,
        height: None,
        fps: None,
        bitrate_kbps: None,
    };
    for pair in params.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k.trim() {
            "resolution" => {
                if let Some((w, h)) = v.split_once('x') {
                    p.width = w.trim().parse().ok();
                    p.height = h.trim().parse().ok();
                }
            }
            "fps" => p.fps = v.trim().parse().ok().filter(|f: &u32| *f > 0),
            "videocodec" => {
                p.codec = match v.trim().to_ascii_lowercase().as_str() {
                    "h264" => Some(VideoCodec::H264),
                    "h265" => Some(VideoCodec::H265),
                    "jpeg" | "mjpeg" => Some(VideoCodec::Mjpeg),
                    _ => None,
                }
            }
            "videomaxbitrate" => p.bitrate_kbps = v.trim().parse().ok().filter(|b: &u32| *b > 0),
            _ => {}
        }
    }
    p
}

#[async_trait]
impl CameraProvider for AxisProvider {
    async fn probe(&self) -> Result<ProbeResult, ProviderError> {
        let (device, response_time_ms) = self.unrestricted_info().await?;
        Ok(ProbeResult {
            response_time_ms,
            device,
        })
    }

    async fn get_device_info(&self) -> Result<DeviceInfo, ProviderError> {
        let (mut device, _) = self.unrestricted_info().await?;
        if self.client()?.has_credentials() {
            match self.network_info().await {
                Ok(net) => {
                    device.hostname = net.hostname;
                    device.mac_address = net.mac;
                    device.network = Some(net.info);
                }
                // Needs an Administrator account; less detail is fine.
                Err(e)
                    if matches!(
                        e.category(),
                        ErrorCategory::AuthFailed | ErrorCategory::Unsupported
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(device)
    }

    async fn get_status(&self) -> Result<CameraStatus, ProviderError> {
        match self
            .call(
                SYSTEM_READY,
                json!({ "apiVersion": "1.0", "method": "systemready", "params": { "timeout": 1 } }),
                Access::Anonymous,
            )
            .await
        {
            Ok((data, response_time_ms)) => {
                if str_field(&data, "systemready").as_deref() == Some("no") {
                    return Err(ProviderError::Protocol(
                        "the device is still starting up".into(),
                    ));
                }
                Ok(CameraStatus {
                    response_time_ms,
                    uptime_seconds: str_field(&data, "uptime").and_then(|u| u.parse().ok()),
                })
            }
            // Before AXIS OS 9.50 there is no systemready; fall back to the
            // anonymous basic device information call.
            Err(e) if e.category() == ErrorCategory::Unsupported => {
                let (_, response_time_ms) = self.unrestricted_info().await?;
                Ok(CameraStatus {
                    response_time_ms,
                    uptime_seconds: None,
                })
            }
            Err(e) => Err(e),
        }
    }

    async fn get_capabilities(&self) -> Result<CameraCapabilities, ProviderError> {
        match self
            .call(
                API_DISCOVERY,
                json!({ "apiVersion": "1.0", "method": "getApiList" }),
                Access::Anonymous,
            )
            .await
        {
            Ok((data, _)) => Ok(capabilities_from_api_list(&data)),
            // API discovery needs AXIS OS 8.50+; older firmware reports nothing.
            Err(e) if e.category() == ErrorCategory::Unsupported => {
                Ok(capabilities_from_api_list(&Value::Null))
            }
            Err(e) => Err(e),
        }
    }

    async fn get_stream_info(&self) -> Result<Vec<StreamProfile>, ProviderError> {
        // Needs at least a Viewer account, so this also verifies the credentials.
        let (data, _) = self
            .call(
                STREAM_PROFILES,
                json!({ "apiVersion": "1.0", "method": "list", "params": { "streamProfileName": [] } }),
                Access::Authenticated,
            )
            .await?;
        let mut profiles: Vec<StreamProfile> = data
            .get("streamProfile")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|sp| {
                        let name = str_field(sp, "name")?;
                        let params = sp.get("parameters").and_then(Value::as_str).unwrap_or("");
                        Some(profile_from(&name, str_field(sp, "description"), params))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if profiles.is_empty() {
            // The camera always has a default stream, even with no saved profiles.
            profiles.push(StreamProfile {
                id: StreamProfileId(String::new()),
                name: "Camera default".into(),
                codec: None,
                width: None,
                height: None,
                fps: None,
                bitrate_kbps: None,
            });
        }
        Ok(profiles)
    }

    async fn open_stream(
        &self,
        profile: &StreamProfileId,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError> {
        crate::streaming::rtsp::open(
            crate::streaming::rtsp::RtspRequest {
                url: self.rtsp_url(profile)?,
                credentials: self.credentials.clone(),
                timeout: self.timeout,
            },
            cancel,
        )
        .await
    }

    async fn restart(&self) -> Result<(), ProviderError> {
        // Firmware management API `reboot` (replaces the obsolete restart.cgi).
        self.call(
            FIRMWARE_MANAGEMENT,
            json!({ "apiVersion": "1.0", "method": "reboot" }),
            Access::Authenticated,
        )
        .await?;
        Ok(())
    }

    fn tls_fingerprint(&self) -> Option<String> {
        self.client
            .as_ref()
            .ok()
            .and_then(VapixClient::seen_fingerprint)
    }
}

#[cfg(test)]
mod rtsp_tests;
#[cfg(test)]
mod tests;
