//! HTTP layer: routes, OpenAPI document, security headers and static files.

pub mod error;
pub mod health;
mod static_files;

use axum::http::{header, HeaderName, HeaderValue};
use axum::Router;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use crate::AppState;

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
    tags((name = "system", description = "Health and server information")),
)]
struct ApiDoc;

/// Versioned API routes with their OpenAPI annotations.
fn v1_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health::health))
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

    // Auth arrives in Phase 1. Until then the docs are always served; afterwards
    // they will be admin-only unless `server.public_api_docs` is set (§13.1).
    let docs = Router::<AppState>::from(SwaggerUi::new(DOCS_PATH).url(OPENAPI_PATH, doc)).layer(
        SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(DOCS_CSP),
        ),
    );

    Router::new()
        .merge(api)
        .merge(docs)
        .fallback(static_files::serve)
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
