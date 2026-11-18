//! Extractor wrappers that turn axum's plain-text rejections into problem+json.

use axum::extract::{FromRequest, FromRequestParts};

use super::error::ApiError;

/// `Json<T>` with problem+json rejections.
#[derive(Debug, FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct JsonBody<T>(pub T);

/// `Path<T>` with problem+json rejections.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct PathParam<T>(pub T);

/// `Query<T>` with problem+json rejections.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct QueryParams<T>(pub T);
