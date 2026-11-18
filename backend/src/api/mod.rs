//! HTTP layer: routes, OpenAPI document, security headers and static files.

mod audit;
mod auth;
pub mod error;
mod extract;
pub mod health;
mod pagination;
mod setup;
mod static_files;
mod users;

use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Router;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use crate::auth::session::COOKIE_NAME;
use crate::auth::AdminUser;
use crate::AppState;
use error::ApiError;

pub const API_V1: &str = "/api/v1";
pub const OPENAPI_PATH: &str = "/api/openapi.json";
pub const DOCS_PATH: &str = "/api/docs";

#[derive(OpenApi)]
#[openapi(
    info(
        title = "CCANH API",
        description = "Crane Camera Automated Navigation Hub. Each operation states its required role.",
    ),
    components(schemas(error::Problem)),
    modifiers(&SessionCookie),
    security(("session_cookie" = [])),
    tags(
        (name = "system", description = "Health and server information"),
        (name = "setup", description = "First-run creation of the initial admin"),
        (name = "auth", description = "Sign in, sign out, own account"),
        (name = "users", description = "User administration"),
        (name = "audit", description = "Security audit log"),
    ),
)]
struct ApiDoc;

/// Documents the session cookie. State-changing requests additionally need an
/// `Origin` header matching the server (CSRF protection, ADR 0004).
struct SessionCookie;

impl Modify for SessionCookie {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                COOKIE_NAME,
                "Session cookie set by POST /api/v1/auth/login",
            ))),
        );
    }
}

/// Versioned API routes with their OpenAPI annotations.
fn v1_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(health::health))
        .routes(routes!(setup::status, setup::complete))
        .routes(routes!(auth::login))
        .routes(routes!(auth::logout))
        .routes(routes!(auth::me))
        .routes(routes!(auth::change_password))
        .routes(routes!(users::list, users::create))
        .routes(routes!(users::get, users::update, users::delete))
        .routes(routes!(users::reset_password))
        .routes(routes!(audit::list))
}

/// Unless `server.public_api_docs` is set, the OpenAPI document and Swagger UI
/// are admin-only (SPEC §13.1). Browsers asking for the docs page while signed
/// out are sent to the login page.
async fn docs_guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if state.config.server.public_api_docs {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    match <AdminUser as axum::extract::FromRequestParts<AppState>>::from_request_parts(
        &mut parts, &state,
    )
    .await
    {
        Ok(AdminUser(_)) => next.run(Request::from_parts(parts, body)).await,
        Err(ApiError::Unauthorized) if parts.uri.path().starts_with(DOCS_PATH) => {
            Redirect::to("/login?next=/api/docs/").into_response()
        }
        Err(err) => err.into_response(),
    }
}

fn split() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(API_V1, v1_routes())
        .split_for_parts()
}

/// The OpenAPI document, built without any runtime state (used by `--print-openapi`).
pub fn openapi() -> utoipa::openapi::OpenApi {
    let mut doc = split().1;
    doc.info.version = env!("CARGO_PKG_VERSION").to_owned();
    doc
}

/// Content Security Policy for the app. No inline scripts or styles.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
    img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; media-src 'self' blob:; \
    worker-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; \
    frame-ancestors 'none'";

/// CSP for Swagger UI only. Its React components set inline `style` attributes,
/// so styles need `'unsafe-inline'`; scripts stay restricted to `'self'`.
const DOCS_CSP: &str = "default-src 'self'; script-src 'self'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; \
    object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// Build the full application router.
pub fn router(state: AppState) -> Router {
    let (api, mut doc) = split();
    doc.info.version = env!("CARGO_PKG_VERSION").to_owned();

    let docs = Router::<AppState>::from(SwaggerUi::new(DOCS_PATH).url(OPENAPI_PATH, doc))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(DOCS_CSP),
        ))
        .layer(middleware::from_fn_with_state(state.clone(), docs_guard));

    Router::new()
        .merge(api)
        .merge(docs)
        .fallback(static_files::serve)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::csrf::guard,
        ))
        .with_state(state)
        // `if_not_present` so the docs router's own CSP wins.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(TraceLayer::new_for_http())
}
