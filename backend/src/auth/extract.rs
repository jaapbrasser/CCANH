//! Request extractors that enforce authentication and roles.
//!
//! Put the auth extractor first in a handler's argument list so it runs before
//! body or path parsing: unauthenticated requests get 401 rather than 400.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use super::session::{self, SessionUser, Timeouts, COOKIE_NAME};
use super::Role;
use crate::api::error::ApiError;
use crate::AppState;

/// Any signed-in, enabled user (viewer or admin).
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
    /// Hash of the current session token, for logout and "other sessions" logic.
    pub session_hash: String,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

impl From<SessionUser> for AuthUser {
    fn from(s: SessionUser) -> Self {
        Self {
            id: s.user_id,
            username: s.username,
            role: s.role,
            session_hash: s.token_hash,
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if let Some(user) = parts.extensions.get::<AuthUser>() {
            return Ok(user.clone());
        }
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(COOKIE_NAME)
            .map(|c| c.value().to_owned())
            .ok_or(ApiError::Unauthorized)?;
        let timeouts = Timeouts::from(&state.config.auth);
        let session = session::validate(&state.db, &token, timeouts)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        let user = AuthUser::from(session);
        parts.extensions.insert(user.clone());
        Ok(user)
    }
}

/// A signed-in admin. Viewers get 403, anonymous requests 401.
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err(ApiError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}

impl std::ops::Deref for AdminUser {
    type Target = AuthUser;
    fn deref(&self) -> &AuthUser {
        &self.0
    }
}

/// The TCP peer address. No proxy headers are trusted: the server is meant to be
/// reached directly on the LAN (SPEC §18 Q5).
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0.ip()),
        ))
    }
}
