//! Camera discovery (SPEC §5): scan IPv4 ranges for AXIS devices as a
//! background task, stream results live, flag duplicates, and register only
//! what an admin approves.

pub mod prober;
pub mod ranges;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio::time::Instant;
use utoipa::ToSchema;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use self::prober::{HostProber, Identified, ProbeOptions, ProbeOutcome};
use self::ranges::Ranges;
use crate::cameras::Scheme;
use crate::events::{Event, EventBus};
use crate::settings::DiscoverySettings;
use crate::tasks::{Task, TaskContext, TaskFailure, TaskResult};

/// Why a result is flagged as a duplicate, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReason {
    SerialNumber,
    MacAddress,
    Ip,
    Hostname,
}

/// What a result duplicates: a registered camera, or an earlier result in
/// the same scan (the same device answering on two addresses).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DuplicateOf {
    pub reason: DuplicateReason,
    pub camera_id: Option<Uuid>,
    pub camera_name: Option<String>,
    pub result_id: Option<Uuid>,
}

/// One device found by a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DiscoveredDevice {
    pub id: Uuid,
    pub job_id: Uuid,
    pub ip: String,
    pub port: u16,
    pub scheme: Scheme,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub mac_address: Option<String>,
    pub hostname: Option<String>,
    pub device_name: Option<String>,
    /// HTTPS certificate fingerprint seen during the scan (unverified).
    /// Registering pins it, after the admin has approved the device.
    pub tls_fingerprint_sha256: Option<String>,
    pub response_time_ms: Option<u32>,
    /// Set when this result was registered from this scan.
    pub registered_camera_id: Option<Uuid>,
    /// Set when the device is already registered or appears earlier in the scan.
    pub duplicate_of: Option<DuplicateOf>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub found_at: OffsetDateTime,
}

/// A discovery job: its ranges, counters and task (status, progress, cancel).
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct DiscoveryJob {
    /// Same as the task id; cancel with `POST /tasks/{id}/cancel`.
    pub id: Uuid,
    pub ranges: Vec<String>,
    pub address_count: u32,
    pub scanned: u32,
    /// Addresses with an open HTTP or HTTPS port.
    pub responsive: u32,
    /// AXIS devices identified.
    pub found: u32,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    pub task: Task,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct DiscoveryJobDetail {
    #[serde(flatten)]
    pub job: DiscoveryJob,
    /// Devices found so far, by address.
    pub results: Vec<DiscoveredDevice>,
}

// ---------------------------------------------------------------------------
// Duplicate detection
// ---------------------------------------------------------------------------

/// What duplicate detection needs to know about a registered camera.
#[derive(Debug, Clone)]
pub struct KnownCamera {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub serial_number: Option<String>,
    pub mac_address: Option<String>,
    pub hostname: Option<String>,
}

/// Hex digits only, uppercase: AXIS serials are MACs, written with or
/// without separators.
fn hw(v: &Option<String>) -> Option<String> {
    let s: String = v
        .as_deref()?
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (!s.is_empty()).then_some(s)
}

fn host_key(v: &Option<String>) -> Option<String> {
    let s = v
        .as_deref()?
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    (!s.is_empty()).then_some(s)
}

/// Match a result against registered cameras, then earlier results.
/// Serial number or MAC beats IP or hostname (SPEC §5.1).
fn duplicate_of(
    d: &DiscoveredDevice,
    cameras: &[KnownCamera],
    earlier: &[DiscoveredDevice],
) -> Option<DuplicateOf> {
    let (serial, mac, host) = (
        hw(&d.serial_number),
        hw(&d.mac_address),
        host_key(&d.hostname),
    );
    let camera = |reason: DuplicateReason, c: &KnownCamera| DuplicateOf {
        reason,
        camera_id: Some(c.id),
        camera_name: Some(c.name.clone()),
        result_id: None,
    };
    let hw_match = |a: &Option<String>, b: &Option<String>| a.is_some() && a == b;
    for c in cameras {
        let (cs, cm) = (hw(&c.serial_number), hw(&c.mac_address));
        // A camera's serial may be recorded as its MAC and vice versa.
        if hw_match(&serial, &cs) || hw_match(&serial, &cm) {
            return Some(camera(DuplicateReason::SerialNumber, c));
        }
        if hw_match(&mac, &cm) || hw_match(&mac, &cs) {
            return Some(camera(DuplicateReason::MacAddress, c));
        }
    }
    for e in earlier {
        if hw_match(&serial, &hw(&e.serial_number)) || hw_match(&mac, &hw(&e.mac_address)) {
            let reason = if hw_match(&serial, &hw(&e.serial_number)) {
                DuplicateReason::SerialNumber
            } else {
                DuplicateReason::MacAddress
            };
            return Some(DuplicateOf {
                reason,
                camera_id: None,
                camera_name: None,
                result_id: Some(e.id),
            });
        }
    }
    for c in cameras {
        if c.host.eq_ignore_ascii_case(&d.ip) {
            return Some(camera(DuplicateReason::Ip, c));
        }
        if host.is_some()
            && (host == host_key(&c.hostname) || host == host_key(&Some(c.host.clone())))
        {
            return Some(camera(DuplicateReason::Hostname, c));
        }
    }
    None
}

/// Flag duplicates across a scan's results, in the order they were found.
pub fn classify(results: &mut [DiscoveredDevice], cameras: &[KnownCamera]) {
    for i in 0..results.len() {
        let (earlier, rest) = results.split_at_mut(i);
        let d = &mut rest[0];
        d.duplicate_of = duplicate_of(d, cameras, earlier);
    }
}

pub async fn known_cameras(db: &SqlitePool) -> Result<Vec<KnownCamera>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id: Hyphenated", name, host, serial_number, mac_address, device_hostname
             FROM cameras"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| KnownCamera {
            id: r.id.into_uuid(),
            name: r.name,
            host: r.host,
            serial_number: r.serial_number,
            mac_address: r.mac_address,
            hostname: r.device_hostname,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

pub async fn create_job(
    db: &SqlitePool,
    id: Uuid,
    ranges: &[String],
    address_count: u32,
) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    let json = serde_json::to_string(ranges).unwrap_or_else(|_| "[]".into());
    let count = i64::from(address_count);
    let now = OffsetDateTime::now_utc();
    sqlx::query!(
        "INSERT INTO discovery_jobs (id, ranges, address_count, created_at) VALUES (?1, ?2, ?3, ?4)",
        hid,
        json,
        count,
        now
    )
    .execute(db)
    .await?;
    Ok(())
}

async fn save_counters(db: &SqlitePool, id: Uuid, c: &Counters) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    let (s, r, f) = (
        i64::from(c.scanned),
        i64::from(c.responsive),
        i64::from(c.found),
    );
    sqlx::query!(
        "UPDATE discovery_jobs SET scanned = ?2, responsive = ?3, found = ?4 WHERE id = ?1",
        hid,
        s,
        r,
        f
    )
    .execute(db)
    .await?;
    Ok(())
}

async fn insert_result(
    db: &SqlitePool,
    job: Uuid,
    ip: Ipv4Addr,
    f: &Identified,
) -> Result<DiscoveredDevice, sqlx::Error> {
    let d = DiscoveredDevice {
        id: Uuid::now_v7(),
        job_id: job,
        ip: ip.to_string(),
        port: f.port,
        scheme: f.scheme,
        manufacturer: f.device.manufacturer.clone(),
        model: f.device.model.clone(),
        firmware_version: f.device.firmware_version.clone(),
        serial_number: f.device.serial_number.clone(),
        mac_address: f.device.mac_address.clone(),
        hostname: f.device.hostname.clone(),
        device_name: f.device.device_name.clone(),
        tls_fingerprint_sha256: f.tls_fingerprint_sha256.clone(),
        response_time_ms: Some(f.response_time_ms),
        registered_camera_id: None,
        duplicate_of: None,
        found_at: OffsetDateTime::now_utc(),
    };
    let (hid, hjob) = (d.id.hyphenated(), job.hyphenated());
    let port = i64::from(d.port);
    let rt = d.response_time_ms.map(i64::from);
    sqlx::query!(
        "INSERT INTO discovery_results (id, job_id, ip, port, scheme, manufacturer, model,
                firmware_version, serial_number, mac_address, hostname, device_name,
                tls_fingerprint_sha256, response_time_ms, found_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        hid,
        hjob,
        d.ip,
        port,
        d.scheme,
        d.manufacturer,
        d.model,
        d.firmware_version,
        d.serial_number,
        d.mac_address,
        d.hostname,
        d.device_name,
        d.tls_fingerprint_sha256,
        rt,
        d.found_at
    )
    .execute(db)
    .await?;
    Ok(d)
}

/// Results of a job (unclassified), in the order found.
async fn results(db: &SqlitePool, job: Uuid) -> Result<Vec<DiscoveredDevice>, sqlx::Error> {
    let hjob = job.hyphenated();
    let rows = sqlx::query!(
        r#"SELECT id AS "id: Hyphenated", job_id AS "job_id: Hyphenated", ip, port,
                  scheme AS "scheme: Scheme", manufacturer, model, firmware_version,
                  serial_number, mac_address, hostname, device_name, tls_fingerprint_sha256,
                  response_time_ms, registered_camera_id AS "registered_camera_id: Hyphenated",
                  found_at AS "found_at: OffsetDateTime"
             FROM discovery_results WHERE job_id = ?1
            ORDER BY found_at, id"#,
        hjob
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DiscoveredDevice {
            id: r.id.into_uuid(),
            job_id: r.job_id.into_uuid(),
            ip: r.ip,
            port: u16::try_from(r.port).unwrap_or(0),
            scheme: r.scheme,
            manufacturer: r.manufacturer,
            model: r.model,
            firmware_version: r.firmware_version,
            serial_number: r.serial_number,
            mac_address: r.mac_address,
            hostname: r.hostname,
            device_name: r.device_name,
            tls_fingerprint_sha256: r.tls_fingerprint_sha256,
            response_time_ms: r.response_time_ms.and_then(|v| u32::try_from(v).ok()),
            registered_camera_id: r.registered_camera_id.map(Hyphenated::into_uuid),
            duplicate_of: None,
            found_at: r.found_at,
        })
        .collect())
}

/// A job's results with duplicates flagged against the current cameras.
pub async fn classified_results(
    db: &SqlitePool,
    job: Uuid,
) -> Result<Vec<DiscoveredDevice>, sqlx::Error> {
    let mut list = results(db, job).await?;
    let cameras = known_cameras(db).await?;
    classify(&mut list, &cameras);
    // A result registered from this scan points at its own camera: that's
    // "registered", not a duplicate.
    for d in &mut list {
        if let (Some(reg), Some(dup)) = (d.registered_camera_id, &d.duplicate_of) {
            if dup.camera_id == Some(reg) {
                d.duplicate_of = None;
            }
        }
    }
    Ok(list)
}

pub async fn mark_registered(
    db: &SqlitePool,
    result: Uuid,
    camera: Uuid,
) -> Result<(), sqlx::Error> {
    let (hr, hc) = (result.hyphenated(), camera.hyphenated());
    sqlx::query!(
        "UPDATE discovery_results SET registered_camera_id = ?2 WHERE id = ?1",
        hr,
        hc
    )
    .execute(db)
    .await?;
    Ok(())
}

struct JobRow {
    id: Hyphenated,
    ranges: String,
    address_count: i64,
    scanned: i64,
    responsive: i64,
    found: i64,
    created_at: OffsetDateTime,
}

async fn to_job(db: &SqlitePool, r: JobRow) -> Result<Option<DiscoveryJob>, sqlx::Error> {
    let id = r.id.into_uuid();
    let Some(task) = crate::tasks::get(db, id).await? else {
        return Ok(None);
    };
    let n = |v: i64| u32::try_from(v).unwrap_or(0);
    Ok(Some(DiscoveryJob {
        id,
        ranges: serde_json::from_str(&r.ranges).unwrap_or_default(),
        address_count: n(r.address_count),
        scanned: n(r.scanned),
        responsive: n(r.responsive),
        found: n(r.found),
        created_at: r.created_at,
        task,
    }))
}

/// Recent jobs, newest first.
pub async fn list_jobs(db: &SqlitePool, limit: i64) -> Result<Vec<DiscoveryJob>, sqlx::Error> {
    let rows = sqlx::query_as!(
        JobRow,
        r#"SELECT id AS "id: Hyphenated", ranges, address_count, scanned, responsive, found,
                  created_at AS "created_at: OffsetDateTime"
             FROM discovery_jobs ORDER BY created_at DESC, id DESC LIMIT ?1"#,
        limit
    )
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        if let Some(j) = to_job(db, r).await? {
            out.push(j);
        }
    }
    Ok(out)
}

pub async fn get_job(db: &SqlitePool, id: Uuid) -> Result<Option<DiscoveryJobDetail>, sqlx::Error> {
    let hid = id.hyphenated();
    let row = sqlx::query_as!(
        JobRow,
        r#"SELECT id AS "id: Hyphenated", ranges, address_count, scanned, responsive, found,
                  created_at AS "created_at: OffsetDateTime"
             FROM discovery_jobs WHERE id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let Some(job) = to_job(db, row).await? else {
        return Ok(None);
    };
    Ok(Some(DiscoveryJobDetail {
        results: classified_results(db, id).await?,
        job,
    }))
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    scanned: u32,
    responsive: u32,
    found: u32,
}

/// How often progress is stored and pushed.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Run a scan as the body of a `discovery` task. Cancellation stops between
/// hosts; results found so far are kept.
pub async fn run(
    db: SqlitePool,
    bus: EventBus,
    prober: Arc<dyn HostProber>,
    ranges: Ranges,
    settings: DiscoverySettings,
    ctx: TaskContext,
) -> TaskResult {
    let job = ctx.id;
    let total = u32::try_from(ranges.count()).unwrap_or(u32::MAX);
    let opts = ProbeOptions {
        connect_timeout: Duration::from_millis(settings.connect_timeout_ms),
        http_timeout: Duration::from_millis(settings.http_timeout_ms),
    };
    let fail = |e: sqlx::Error| TaskFailure::internal(format!("database: {e}"));
    let cameras = known_cameras(&db).await.map_err(fail)?;
    let mut found_so_far: Vec<DiscoveredDevice> = Vec::new();
    let mut c = Counters::default();
    let mut last_progress = Instant::now();

    let concurrency = usize::try_from(settings.concurrency.max(1)).unwrap_or(32);
    let addresses: Vec<Ipv4Addr> = ranges.addresses().collect();
    let mut probes = futures_util::stream::iter(addresses)
        .map(|ip| {
            let prober = Arc::clone(&prober);
            async move { (ip, prober.probe(ip, &opts).await) }
        })
        .buffer_unordered(concurrency);

    ctx.progress(0, Some(total), "Scanning").await;
    loop {
        let next = tokio::select! {
            () = ctx.cancel.cancelled() => break,
            n = probes.next() => n,
        };
        let Some((ip, outcome)) = next else { break };
        c.scanned += 1;
        match outcome {
            ProbeOutcome::NoService => {}
            ProbeOutcome::Unidentified => c.responsive += 1,
            ProbeOutcome::Found(f) => {
                c.responsive += 1;
                c.found += 1;
                let mut d = insert_result(&db, job, ip, &f).await.map_err(fail)?;
                d.duplicate_of = duplicate_of(&d, &cameras, &found_so_far);
                found_so_far.push(d.clone());
                bus.publish(Event::DiscoveryFound {
                    job_id: job,
                    result: d,
                });
            }
        }
        if last_progress.elapsed() >= PROGRESS_EVERY || c.scanned == total {
            last_progress = Instant::now();
            report(&db, &bus, &ctx, job, total, &c)
                .await
                .map_err(fail)?;
        }
    }
    report(&db, &bus, &ctx, job, total, &c)
        .await
        .map_err(fail)?;
    Ok(serde_json::json!({
        "scanned": c.scanned, "responsive": c.responsive, "found": c.found, "address_count": total,
    }))
}

async fn report(
    db: &SqlitePool,
    bus: &EventBus,
    ctx: &TaskContext,
    job: Uuid,
    total: u32,
    c: &Counters,
) -> Result<(), sqlx::Error> {
    save_counters(db, job, c).await?;
    let message = format!(
        "Scanned {} of {total}: {} responding, {} AXIS device{} found",
        c.scanned,
        c.responsive,
        c.found,
        if c.found == 1 { "" } else { "s" }
    );
    ctx.progress(c.scanned, Some(total), &message).await;
    bus.publish(Event::DiscoveryProgress {
        job_id: job,
        scanned: c.scanned,
        total,
        responsive: c.responsive,
        found: c.found,
    });
    Ok(())
}

/// Index results by id (registration helper).
pub fn by_id(list: Vec<DiscoveredDevice>) -> HashMap<Uuid, DiscoveredDevice> {
    list.into_iter().map(|d| (d.id, d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(
        ip: &str,
        serial: Option<&str>,
        mac: Option<&str>,
        host: Option<&str>,
    ) -> DiscoveredDevice {
        DiscoveredDevice {
            id: Uuid::now_v7(),
            job_id: Uuid::nil(),
            ip: ip.into(),
            port: 443,
            scheme: Scheme::Https,
            manufacturer: Some("AXIS".into()),
            model: None,
            firmware_version: None,
            serial_number: serial.map(Into::into),
            mac_address: mac.map(Into::into),
            hostname: host.map(Into::into),
            device_name: None,
            tls_fingerprint_sha256: None,
            response_time_ms: None,
            registered_camera_id: None,
            duplicate_of: None,
            found_at: OffsetDateTime::now_utc(),
        }
    }

    fn camera(
        host: &str,
        serial: Option<&str>,
        mac: Option<&str>,
        hostname: Option<&str>,
    ) -> KnownCamera {
        KnownCamera {
            id: Uuid::now_v7(),
            name: format!("Cam {host}"),
            host: host.into(),
            serial_number: serial.map(Into::into),
            mac_address: mac.map(Into::into),
            hostname: hostname.map(Into::into),
        }
    }

    #[test]
    fn serial_and_mac_beat_address_matches() {
        let cams = [
            camera("192.0.2.11", Some("ACCC8E000011"), None, None),
            camera("192.0.2.50", None, Some("AC:CC:8E:00:00:22"), None),
            camera("192.0.2.60", None, None, Some("axis-gate.example")),
        ];
        let mut list = vec![
            // Moved to a new address but same serial (separators differ).
            device("192.0.2.99", Some("ac:cc:8e:00:00:11"), None, None),
            // Serial recorded as MAC on the camera.
            device("192.0.2.98", Some("ACCC8E000022"), None, None),
            device("192.0.2.50", Some("FFFFFFFFFFFF"), None, None),
            device("192.0.2.97", None, None, Some("AXIS-GATE.example.")),
            device("192.0.2.96", Some("ACCC8E000096"), None, None),
            // Same device answering on a second address.
            device("192.0.2.95", Some("ACCC8E000096"), None, None),
        ];
        classify(&mut list, &cams);
        let reasons: Vec<_> = list
            .iter()
            .map(|d| d.duplicate_of.as_ref().map(|x| x.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                Some(DuplicateReason::SerialNumber),
                Some(DuplicateReason::SerialNumber),
                Some(DuplicateReason::Ip),
                Some(DuplicateReason::Hostname),
                None,
                Some(DuplicateReason::SerialNumber),
            ]
        );
        assert_eq!(
            list[0].duplicate_of.as_ref().unwrap().camera_id,
            Some(cams[0].id)
        );
        assert_eq!(
            list[5].duplicate_of.as_ref().unwrap().result_id,
            Some(list[4].id)
        );
    }
}
