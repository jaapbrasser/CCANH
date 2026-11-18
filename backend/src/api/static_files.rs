//! Serves the frontend single-page app.
//!
//! Release builds embed `frontend/dist` into the binary. Debug builds read it from
//! disk at request time (rust-embed's debug behaviour), so `npm run build` output is
//! picked up without recompiling. If the frontend hasn't been built, a placeholder
//! page explains how to run the dev server instead of failing.

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

use super::error::ApiError;

#[derive(RustEmbed)]
#[folder = "../frontend/dist/"]
#[allow_missing = true]
struct Assets;

const PLACEHOLDER: &str = include_str!("placeholder.html");

/// Fallback handler for every path not matched by the API router.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Unknown API paths get a problem+json 404, never the SPA shell.
    if path == "api" || path.starts_with("api/") {
        return ApiError::NotFound.into_response();
    }

    if !path.is_empty() {
        if let Some(file) = Assets::get(path) {
            return file_response(path, file);
        }
        // Requests for missing static files (anything with an extension) are 404s;
        // everything else is a client-side route and gets index.html.
        if path.rsplit('/').next().is_some_and(|seg| seg.contains('.')) {
            return StatusCode::NOT_FOUND.into_response();
        }
    }

    match Assets::get("index.html") {
        Some(file) => file_response("index.html", file),
        None => Html(PLACEHOLDER).into_response(),
    }
}

fn file_response(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    let mime = file.metadata.mimetype();
    // Vite fingerprints everything under assets/, so it can be cached forever.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = Response::new(Body::from(file.data.into_owned()));
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}
