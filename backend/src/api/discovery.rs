//! Camera discovery (SPEC §5). Admin only. Scans run as `discovery` tasks;
//! results are registered only when an admin approves them.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{JsonBody, PathParam};
use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::auth::{AdminUser, ClientIp};
use crate::cameras::{
    self, CameraSettings, CameraSummary, CredentialInput, NewCamera, Scheme, TlsMode,
};
use crate::discovery::{self, prober, ranges::Ranges, DiscoveryJob, DiscoveryJobDetail};
use crate::events::Event;
use crate::organization::{groups, tags};
use crate::secret::{ExposeSecret, SecretString};
use crate::settings::MAX_SCAN_ADDRESSES;
use crate::tasks::Creator;
use crate::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct DiscoveryRanges {
    /// IPv4 CIDR blocks, single addresses or dash ranges
    /// (`192.0.2.0/24`, `192.0.2.7`, `192.0.2.10-20`). Entries may hold
    /// several, separated by commas, spaces or newlines.
    pub ranges: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DiscoveryPreview {
    pub address_count: u64,
    /// The configured cap (runtime setting `discovery.max_addresses`).
    pub max_addresses: u32,
    /// More than the cap: starting needs `confirm_large: true`.
    pub needs_confirmation: bool,
    /// More than the hard ceiling: can't be scanned at all.
    pub too_large: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartDiscovery {
    pub ranges: Vec<String>,
    /// Required to scan more addresses than `discovery.max_addresses`.
    #[serde(default)]
    pub confirm_large: bool,
}

fn parse(inputs: &[String]) -> Result<Ranges, ApiError> {
    Ranges::parse(inputs).map_err(|e| ApiError::BadRequest(e.to_string()))
}

fn preview_of(state: &AppState, ranges: &Ranges) -> DiscoveryPreview {
    let max = state.settings.current().discovery.max_addresses;
    let n = ranges.count();
    DiscoveryPreview {
        address_count: n,
        max_addresses: max,
        needs_confirmation: n > u64::from(max),
        too_large: n > u64::from(MAX_SCAN_ADDRESSES),
    }
}

/// Count the addresses ranges cover and check them against the caps,
/// without starting anything.
///
/// Required role: admin.
#[utoipa::path(
    post,
    path = "/discovery/preview",
    operation_id = "preview_discovery",
    tag = "discovery",
    request_body = DiscoveryRanges,
    responses(
        (status = 200, description = "Address count and caps", body = DiscoveryPreview),
        (status = 400, description = "A range can't be parsed", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn preview(
    _admin: AdminUser,
    State(state): State<AppState>,
    JsonBody(body): JsonBody<DiscoveryRanges>,
) -> Result<Json<DiscoveryPreview>, ApiError> {
    let ranges = parse(&body.ranges)?;
    Ok(Json(preview_of(&state, &ranges)))
}

/// Start scanning ranges for AXIS devices.
///
/// Required role: admin. Runs as a `discovery` task (202). Results and
/// progress stream on the `discovery` WebSocket topic; cancel with
/// `POST /tasks/{id}/cancel`. Nothing is registered automatically. Audited.
#[utoipa::path(
    post,
    path = "/discovery/jobs",
    operation_id = "start_discovery",
    tag = "discovery",
    request_body = StartDiscovery,
    responses(
        (status = 202, description = "Scan started", body = DiscoveryJob),
        (status = 400, description = "Invalid ranges, or too many addresses without confirmation", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn start(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<StartDiscovery>,
) -> Result<(StatusCode, Json<DiscoveryJob>), ApiError> {
    let ranges = parse(&body.ranges)?;
    let p = preview_of(&state, &ranges);
    if p.too_large {
        return Err(ApiError::BadRequest(format!(
            "These ranges cover {} addresses; one scan can cover at most {MAX_SCAN_ADDRESSES}.",
            p.address_count
        )));
    }
    if p.needs_confirmation && !body.confirm_large {
        return Err(ApiError::BadRequest(format!(
            "These ranges cover {} addresses, more than the limit of {}. Confirm to scan them anyway.",
            p.address_count, p.max_addresses
        )));
    }
    let count = u32::try_from(p.address_count).unwrap_or(u32::MAX);
    let settings = state.settings.current().discovery;
    let prober = prober::for_mode(state.config.mode);
    let (db, bus) = (state.db.clone(), state.events.clone());
    // The job row must exist before the scan stores results.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let creator = Creator {
        id: admin.id,
        username: admin.username.clone(),
    };
    let task = state
        .tasks
        .spawn("discovery", None, Some(creator), move |ctx| async move {
            if ready_rx.await.is_err() {
                return Err(crate::tasks::TaskFailure::internal(
                    "the scan could not be recorded",
                ));
            }
            discovery::run(db, bus, prober, ranges, settings, ctx).await
        })
        .await?;
    let created = discovery::create_job(&state.db, task.id, &body.ranges, count).await;
    // Dropping the sender (on error) makes the task fail instead of hanging.
    if created.is_ok() {
        let _ = ready_tx.send(());
    }
    created?;
    audit::record(
        &state.db,
        AuditEvent::new(action::DISCOVERY_START, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("discovery", task.id)
            .ip(ip)
            .details(serde_json::json!({
                "ranges": body.ranges, "address_count": count, "confirmed_large": body.confirm_large,
            })),
    )
    .await?;
    let job = discovery::get_job(&state.db, task.id)
        .await?
        .ok_or(ApiError::NotFound)?
        .job;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// Recent discovery jobs, newest first (up to 20).
///
/// Required role: admin.
#[utoipa::path(
    get,
    path = "/discovery/jobs",
    operation_id = "list_discovery_jobs",
    tag = "discovery",
    responses(
        (status = 200, description = "Recent jobs", body = Vec<DiscoveryJob>),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DiscoveryJob>>, ApiError> {
    Ok(Json(discovery::list_jobs(&state.db, 20).await?))
}

/// A job with its results, duplicates flagged against current cameras.
///
/// Required role: admin.
#[utoipa::path(
    get,
    path = "/discovery/jobs/{id}",
    operation_id = "get_discovery_job",
    tag = "discovery",
    params(("id" = Uuid, Path, description = "Job (task) ID")),
    responses(
        (status = 200, description = "The job and its results", body = DiscoveryJobDetail),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such job", body = Problem),
    )
)]
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<DiscoveryJobDetail>, ApiError> {
    discovery::get_job(&state.db, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterDiscovered {
    /// Results to register (up to 500).
    pub result_ids: Vec<Uuid>,
    /// Credentials for all of them (none, a shared set, or inline: each
    /// camera then gets its own private copy).
    #[serde(default)]
    pub credentials: Option<CredentialInput>,
    #[serde(default)]
    pub group_ids: Vec<Uuid>,
    #[serde(default)]
    pub tag_ids: Vec<Uuid>,
    #[serde(default = "default_true")]
    pub monitoring_enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegisteredResult {
    pub result_id: Uuid,
    pub camera: CameraSummary,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SkippedResult {
    pub result_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterOutcome {
    pub registered: Vec<RegisteredResult>,
    pub skipped: Vec<SkippedResult>,
}

fn copy_credentials(c: &Option<CredentialInput>) -> Option<CredentialInput> {
    c.as_ref().map(|c| match c {
        CredentialInput::None => CredentialInput::None,
        CredentialInput::Set { id } => CredentialInput::Set { id: *id },
        CredentialInput::Inline { username, password } => CredentialInput::Inline {
            username: username.clone(),
            password: SecretString::from(password.expose_secret().to_owned()),
        },
    })
}

/// Register approved results as cameras.
///
/// Required role: admin. Results already registered (from this scan or as a
/// duplicate of an existing camera) are skipped with a reason. HTTPS devices
/// are pinned to the certificate fingerprint seen during the scan. Each new
/// camera is audited.
#[utoipa::path(
    post,
    path = "/discovery/jobs/{id}/register",
    operation_id = "register_discovered",
    tag = "discovery",
    params(("id" = Uuid, Path, description = "Job (task) ID")),
    request_body = RegisterDiscovered,
    responses(
        (status = 200, description = "What was registered and what was skipped", body = RegisterOutcome),
        (status = 400, description = "Invalid selection, groups or tags", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such job", body = Problem),
    )
)]
pub async fn register(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<RegisterDiscovered>,
) -> Result<Json<RegisterOutcome>, ApiError> {
    if body.result_ids.is_empty() || body.result_ids.len() > 500 {
        return Err(ApiError::BadRequest(
            "select between 1 and 500 results".into(),
        ));
    }
    let org = super::organization::org_error;
    groups::ensure_exist(&state.db, &body.group_ids)
        .await
        .map_err(org)?;
    tags::ensure_exist(&state.db, &body.tag_ids)
        .await
        .map_err(org)?;
    if discovery::get_job(&state.db, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let results = discovery::by_id(discovery::classified_results(&state.db, id).await?);

    let mut out = RegisterOutcome {
        registered: Vec::new(),
        skipped: Vec::new(),
    };
    for rid in crate::organization::unique(&body.result_ids) {
        let skip = |reason: String| SkippedResult {
            result_id: rid,
            reason,
        };
        let Some(d) = results.get(&rid) else {
            out.skipped.push(skip("not a result of this scan".into()));
            continue;
        };
        if d.registered_camera_id.is_some() {
            out.skipped
                .push(skip("already registered from this scan".into()));
            continue;
        }
        if let Some(dup) = &d.duplicate_of {
            let reason = match &dup.camera_name {
                Some(name) => format!("already registered as \"{name}\""),
                None => "the same device appears earlier in this scan".into(),
            };
            out.skipped.push(skip(reason));
            continue;
        }
        let name = d
            .device_name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("{} {}", d.model.as_deref().unwrap_or("AXIS camera"), d.ip));
        let https = d.scheme == Scheme::Https;
        let pin = https.then(|| d.tls_fingerprint_sha256.clone()).flatten();
        let new = NewCamera {
            name: name.chars().take(cameras::MAX_NAME_LEN).collect(),
            host: d.ip.clone(),
            port: Some(d.port),
            scheme: d.scheme,
            tls_mode: if pin.is_some() {
                TlsMode::Pinned
            } else {
                TlsMode::Strict
            },
            tls_pinned_sha256: pin,
            monitoring_enabled: body.monitoring_enabled,
            credentials: copy_credentials(&body.credentials),
            allow_basic_over_http: false,
            group_ids: Vec::new(),
            tag_ids: Vec::new(),
        };
        let (settings, credentials) = match CameraSettings::from_new(new) {
            Ok(v) => v,
            Err(e) => {
                out.skipped.push(skip(e.to_string()));
                continue;
            }
        };
        let created =
            match cameras::create(&state.db, &*state.credentials, &settings, credentials).await {
                Ok(c) => c,
                Err(cameras::CameraError::Duplicate) => {
                    out.skipped.push(skip(
                        "a camera with this address and port already exists".into(),
                    ));
                    continue;
                }
                Err(e) => return Err(super::cameras::camera_error(e)),
            };
        let cid = created.summary.id;
        if !body.group_ids.is_empty() {
            groups::set_for_camera(&state.db, cid, &body.group_ids)
                .await
                .map_err(org)?;
        }
        if !body.tag_ids.is_empty() {
            tags::set_for_camera(&state.db, cid, &body.tag_ids)
                .await
                .map_err(org)?;
        }
        discovery::mark_registered(&state.db, rid, cid).await?;
        let camera = cameras::get(&state.db, cid)
            .await?
            .ok_or(ApiError::NotFound)?;
        let mut details = super::cameras::settings_json(&camera);
        if let Some(obj) = details.as_object_mut() {
            obj.insert("source".into(), serde_json::json!("discovery"));
            obj.insert("discovery_job".into(), serde_json::json!(id));
        }
        audit::record(
            &state.db,
            AuditEvent::new(action::CAMERA_CREATE, Outcome::Success)
                .actor(Some(admin.id), &admin.username)
                .target("camera", cid)
                .ip(ip)
                .details(details),
        )
        .await?;
        state.events.publish(Event::CameraChanged {
            camera: camera.summary.clone(),
        });
        out.registered.push(RegisteredResult {
            result_id: rid,
            camera: camera.summary,
        });
    }
    Ok(Json(out))
}
