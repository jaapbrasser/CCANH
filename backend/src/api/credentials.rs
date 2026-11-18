//! `/api/v1/credential-sets/*` (shared camera credentials, admin only) and
//! `/api/v1/cameras/test-connection`.
//!
//! Credentials are write-only: no response ever contains a username or
//! password (SPEC §4.6).

use std::time::Duration;

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
    self, normalize_fingerprint, tls_policy, validate_host, validate_port, CredentialInput, Scheme,
    TlsMode,
};
use crate::credentials::{self as creds, CredentialSetError, CredentialSetSummary, Credentials};
use crate::providers::{
    CameraProvider, Capability, DeviceInfo, ErrorCategory, ProviderError, ProviderTarget,
    StreamProfile,
};
use crate::secret::SecretString;
use crate::AppState;

pub(crate) fn credential_error(err: CredentialSetError) -> ApiError {
    match err {
        CredentialSetError::NotFound => ApiError::NotFound,
        CredentialSetError::DuplicateName => {
            ApiError::Conflict("A credential set with this name already exists.".into())
        }
        CredentialSetError::InUse(n) => ApiError::Conflict(format!(
            "This credential set is used by {n} camera(s). Assign them other credentials first."
        )),
        CredentialSetError::InvalidName => {
            ApiError::BadRequest("Name must be 1–100 characters.".into())
        }
        CredentialSetError::InvalidUsername => {
            ApiError::BadRequest("Username must be 1–128 characters.".into())
        }
        CredentialSetError::Crypto(e) => ApiError::Internal(format!("credential store: {e}")),
        CredentialSetError::Database(e) => ApiError::from(e),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewCredentialSet {
    pub name: String,
    pub username: String,
    #[schema(value_type = String, format = Password)]
    pub password: SecretString,
}

/// Rename a set and/or replace its credentials (username and password together).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CredentialSetPatch {
    pub name: Option<String>,
    pub username: Option<String>,
    #[schema(value_type = Option<String>, format = Password)]
    pub password: Option<SecretString>,
}

/// List shared credential sets.
///
/// Required role: admin. Never returns usernames or passwords.
#[utoipa::path(
    get,
    path = "/credential-sets",
    operation_id = "list_credential_sets",
    tag = "credentials",
    responses(
        (status = 200, description = "Credential sets", body = Vec<CredentialSetSummary>),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<CredentialSetSummary>>, ApiError> {
    Ok(Json(creds::list_shared(&state.db).await?))
}

/// Create a shared credential set.
///
/// Required role: admin. The credentials are encrypted before storage.
#[utoipa::path(
    post,
    path = "/credential-sets",
    operation_id = "create_credential_set",
    tag = "credentials",
    request_body = NewCredentialSet,
    responses(
        (status = 201, description = "Created", body = CredentialSetSummary),
        (status = 400, description = "Invalid input", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 409, description = "Name taken", body = Problem),
    )
)]
pub async fn create(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<NewCredentialSet>,
) -> Result<(StatusCode, Json<CredentialSetSummary>), ApiError> {
    let set = creds::create_shared(
        &state.db,
        &*state.credentials,
        &body.name,
        &Credentials {
            username: body.username,
            password: body.password,
        },
    )
    .await
    .map_err(credential_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::CREDENTIAL_SET_CREATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("credential_set", set.id)
            .ip(ip)
            .details(serde_json::json!({ "name": set.name })),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(set)))
}

/// Rename a credential set or replace its credentials.
///
/// Required role: admin. Cameras using the set pick up new credentials on
/// their next check.
#[utoipa::path(
    patch,
    path = "/credential-sets/{id}",
    operation_id = "update_credential_set",
    tag = "credentials",
    params(("id" = Uuid, Path, description = "Credential set ID")),
    request_body = CredentialSetPatch,
    responses(
        (status = 200, description = "Updated", body = CredentialSetSummary),
        (status = 400, description = "Invalid input", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such set", body = Problem),
        (status = 409, description = "Name taken", body = Problem),
    )
)]
pub async fn update(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<CredentialSetPatch>,
) -> Result<Json<CredentialSetSummary>, ApiError> {
    let new_creds = match (body.username, body.password) {
        (Some(username), Some(password)) => Some(Credentials { username, password }),
        (None, None) => None,
        _ => {
            return Err(ApiError::BadRequest(
                "Send username and password together to replace credentials.".into(),
            ))
        }
    };
    let set = creds::update_shared(
        &state.db,
        &*state.credentials,
        id,
        body.name.as_deref(),
        new_creds.as_ref(),
    )
    .await
    .map_err(credential_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::CREDENTIAL_SET_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("credential_set", id)
            .ip(ip)
            .details(serde_json::json!({
                "name": set.name,
                "credentials_replaced": new_creds.is_some(),
            })),
    )
    .await?;
    Ok(Json(set))
}

/// Delete a credential set.
///
/// Required role: admin. Refused while any camera uses it.
#[utoipa::path(
    delete,
    path = "/credential-sets/{id}",
    operation_id = "delete_credential_set",
    tag = "credentials",
    params(("id" = Uuid, Path, description = "Credential set ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such set", body = Problem),
        (status = 409, description = "In use", body = Problem),
    )
)]
pub async fn delete(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    let set = creds::get_shared(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    creds::delete_shared(&state.db, id)
        .await
        .map_err(credential_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::CREDENTIAL_SET_DELETE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("credential_set", id)
            .ip(ip)
            .details(serde_json::json!({ "name": set.name })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Test connection
// ---------------------------------------------------------------------------

/// Overall time limit for a connection test.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize, ToSchema)]
pub struct TestConnectionRequest {
    pub host: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub scheme: Scheme,
    #[serde(default)]
    pub tls_mode: TlsMode,
    pub tls_pinned_sha256: Option<String>,
    #[serde(default)]
    pub allow_basic_over_http: bool,
    /// Credentials to test. Omit (or send `null`) together with `camera_id` to
    /// test with an existing camera's stored credentials.
    pub credentials: Option<CredentialInput>,
    /// When editing an existing camera: use its stored credentials if
    /// `credentials` is omitted.
    pub camera_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestStep {
    /// `reachability`, `credentials`, `capabilities` or `device_info`.
    pub name: String,
    pub ok: bool,
    /// `true` when the step doesn't apply (e.g. no credentials to verify).
    pub skipped: bool,
    pub error_category: Option<ErrorCategory>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestConnectionResult {
    /// True when every non-skipped step succeeded.
    pub ok: bool,
    pub response_time_ms: Option<u32>,
    pub device: Option<DeviceInfo>,
    pub capabilities: Vec<Capability>,
    pub stream_profiles: Vec<StreamProfile>,
    /// Fingerprint of the certificate the camera presented (HTTPS only), so it
    /// can be checked and pinned.
    pub tls_fingerprint_sha256: Option<String>,
    pub steps: Vec<TestStep>,
}

fn step<T>(name: &str, r: &Result<T, ProviderError>) -> TestStep {
    match r {
        Ok(_) => TestStep {
            name: name.into(),
            ok: true,
            skipped: false,
            error_category: None,
            message: None,
        },
        Err(e) => TestStep {
            name: name.into(),
            ok: false,
            skipped: false,
            error_category: Some(e.category()),
            message: Some(e.to_string()),
        },
    }
}

async fn run_test(provider: &dyn CameraProvider, has_credentials: bool) -> TestConnectionResult {
    let probe = provider.probe().await;
    let mut steps = vec![step("reachability", &probe)];
    let reachable = probe.is_ok();
    let mut result = TestConnectionResult {
        ok: false,
        response_time_ms: probe.as_ref().ok().map(|p| p.response_time_ms),
        device: probe.ok().map(|p| p.device),
        capabilities: Vec::new(),
        stream_profiles: Vec::new(),
        tls_fingerprint_sha256: provider.tls_fingerprint(),
        steps: Vec::new(),
    };
    if !reachable {
        steps.push(TestStep {
            name: "credentials".into(),
            ok: false,
            skipped: true,
            error_category: None,
            message: Some("Skipped: the camera is not reachable.".into()),
        });
        result.steps = steps;
        return result;
    }

    // Stream profiles need at least a Viewer account: this verifies credentials.
    let streams = provider.get_stream_info().await;
    let mut cred_step = step("credentials", &streams);
    if !has_credentials && streams.is_err() {
        cred_step.message =
            Some("No credentials configured; the camera requires them for this request.".into());
    }
    steps.push(cred_step);
    result.stream_profiles = streams.unwrap_or_default();

    let caps = provider.get_capabilities().await;
    steps.push(step("capabilities", &caps));
    result.capabilities = caps.map(|c| c.items).unwrap_or_default();

    let info = provider.get_device_info().await;
    steps.push(step("device_info", &info));
    if let Ok(info) = info {
        result.device = Some(info);
    }

    result.ok = steps.iter().all(|s| s.ok || s.skipped);
    result.tls_fingerprint_sha256 = provider.tls_fingerprint();
    result.steps = steps;
    result
}

/// Test connecting to a camera without saving anything.
///
/// Required role: admin. Probes reachability and identity, verifies the
/// credentials, and reads capabilities and stream profiles. Each step reports
/// its own error category. The camera's certificate fingerprint is returned so
/// it can be pinned.
#[utoipa::path(
    post,
    path = "/cameras/test-connection",
    operation_id = "test_camera_connection",
    tag = "cameras",
    request_body = TestConnectionRequest,
    responses(
        (status = 200, description = "Test result (see `ok` and `steps`)", body = TestConnectionResult),
        (status = 400, description = "Invalid settings", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "Unknown camera_id", body = Problem),
    )
)]
pub async fn test_connection(
    _admin: AdminUser,
    State(state): State<AppState>,
    JsonBody(body): JsonBody<TestConnectionRequest>,
) -> Result<Json<TestConnectionResult>, ApiError> {
    let host = validate_host(&body.host).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let port = validate_port(body.port.unwrap_or_else(|| body.scheme.default_port()))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let pin = match (body.tls_mode, body.tls_pinned_sha256.as_deref()) {
        (TlsMode::Pinned, Some(fp)) => {
            Some(normalize_fingerprint(fp).map_err(|e| ApiError::BadRequest(e.to_string()))?)
        }
        (TlsMode::Pinned, None) => {
            return Err(ApiError::BadRequest(
                "tls_mode \"pinned\" requires tls_pinned_sha256".into(),
            ))
        }
        _ => None,
    };

    let credentials = match body.credentials {
        Some(CredentialInput::None) => None,
        Some(CredentialInput::Inline { username, password }) => {
            Some(Credentials { username, password })
        }
        Some(CredentialInput::Set { id }) => {
            if !creds::is_shared(&state.db, id).await? {
                return Err(ApiError::BadRequest(
                    "The selected credential set does not exist.".into(),
                ));
            }
            let secret = creds::secret(&state.db, id)
                .await?
                .ok_or(ApiError::NotFound)?;
            Some(
                state
                    .credentials
                    .open(&secret)
                    .map_err(|e| ApiError::Internal(format!("credential store: {e}")))?,
            )
        }
        None => match body.camera_id {
            Some(cid) => {
                let target = cameras::target(&state.db, cid)
                    .await?
                    .ok_or(ApiError::NotFound)?;
                target
                    .provider_target(&*state.credentials)
                    .map_err(|e| ApiError::Internal(format!("credential store: {e}")))?
                    .credentials
            }
            None => None,
        },
    };

    let has_credentials = credentials.is_some();
    let target = ProviderTarget {
        host,
        port,
        https: body.scheme == Scheme::Https,
        tls: tls_policy(body.tls_mode, pin.as_deref()),
        credentials,
        allow_basic_over_http: body.allow_basic_over_http,
    };
    let provider = state.providers.create(&target);
    let result = tokio::time::timeout(TEST_TIMEOUT, run_test(&*provider, has_credentials))
        .await
        .unwrap_or_else(|_| TestConnectionResult {
            ok: false,
            response_time_ms: None,
            device: None,
            capabilities: Vec::new(),
            stream_profiles: Vec::new(),
            tls_fingerprint_sha256: provider.tls_fingerprint(),
            steps: vec![TestStep {
                name: "reachability".into(),
                ok: false,
                skipped: false,
                error_category: Some(ErrorCategory::Timeout),
                message: Some(format!(
                    "The test did not finish within {} seconds.",
                    TEST_TIMEOUT.as_secs()
                )),
            }],
        });
    Ok(Json(result))
}
