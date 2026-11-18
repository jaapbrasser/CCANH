//! HTTP client for VAPIX requests to one camera.
//!
//! - Every request has a timeout; errors are mapped to [`ProviderError`]
//!   categories (SPEC §10.4).
//! - The authentication scheme is learned from the camera's `WWW-Authenticate`
//!   challenge, never assumed (SPEC §4.5). Digest is preferred when offered;
//!   Basic is used only over HTTPS unless the admin allowed it for this camera.
//! - Credentials are never placed in URLs, logged, or included in error text.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderValue, AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Method, StatusCode};
use serde_json::Value;

use super::digest::{
    best_digest, digest_authorization, parse_challenges, Challenge, DigestChallenge,
};
use super::tls::{client_config, SeenFingerprint};
use crate::credentials::Credentials;
use crate::providers::{ProviderError, ProviderTarget};
use crate::secret::ExposeSecret;

#[derive(Debug, Clone)]
enum CachedAuth {
    Basic,
    Digest { challenge: DigestChallenge, nc: u32 },
}

pub struct VapixClient {
    http: reqwest::Client,
    base: String,
    https: bool,
    host: String,
    creds: Option<Credentials>,
    allow_basic_over_http: bool,
    auth: Mutex<Option<CachedAuth>>,
    seen_fingerprint: SeenFingerprint,
}

/// The raw response of a VAPIX call.
struct Reply {
    body: String,
    elapsed: Duration,
}

/// Whether a request should carry credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Documented as anonymous: send without credentials, but retry with them
    /// if the camera asks (some firmware requires auth anyway).
    Anonymous,
    Authenticated,
}

impl VapixClient {
    pub fn new(target: &ProviderTarget, timeout: Duration) -> Result<Self, ProviderError> {
        let (tls, seen_fingerprint) = client_config(&target.tls)
            .map_err(|e| ProviderError::Internal(format!("TLS setup failed: {e}")))?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(5)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("CCANH/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ProviderError::Internal(format!("HTTP client setup failed: {e}")))?;
        let scheme = if target.https { "https" } else { "http" };
        let host = if target.host.contains(':') {
            format!("[{}]", target.host) // IPv6
        } else {
            target.host.clone()
        };
        Ok(Self {
            http,
            base: format!("{scheme}://{host}:{}", target.port),
            https: target.https,
            host: target.host.clone(),
            creds: target.credentials.clone(),
            allow_basic_over_http: target.allow_basic_over_http,
            auth: Mutex::new(None),
            seen_fingerprint,
        })
    }

    /// SHA-256 fingerprint of the certificate seen on the last HTTPS connection.
    pub fn seen_fingerprint(&self) -> Option<String> {
        self.seen_fingerprint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn has_credentials(&self) -> bool {
        self.creds.is_some()
    }

    fn cached(&self) -> Option<CachedAuth> {
        self.auth.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn cache(&self, a: Option<CachedAuth>) {
        *self.auth.lock().unwrap_or_else(|e| e.into_inner()) = a;
    }

    /// Build the Authorization header for the cached scheme, bumping the Digest
    /// nonce count.
    fn authorization(&self, method: &Method, path: &str) -> Option<String> {
        let creds = self.creds.as_ref()?;
        let mut guard = self.auth.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut()? {
            CachedAuth::Basic => {
                use base64::Engine;
                let raw = format!("{}:{}", creds.username, creds.password.expose_secret());
                Some(format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(raw)
                ))
            }
            CachedAuth::Digest { challenge, nc } => {
                *nc += 1;
                let mut cnonce = [0u8; 12];
                let _ = getrandom::fill(&mut cnonce);
                let cnonce: String = cnonce.iter().map(|b| format!("{b:02x}")).collect();
                Some(digest_authorization(
                    challenge,
                    &creds.username,
                    creds.password.expose_secret(),
                    method.as_str(),
                    path,
                    *nc,
                    &cnonce,
                ))
            }
        }
    }

    /// Choose a scheme from a 401 challenge.
    fn adopt_challenge(&self, headers: &reqwest::header::HeaderMap) -> Result<(), ProviderError> {
        let values: Vec<&str> = headers
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        let challenges = parse_challenges(values);
        if let Some(d) = best_digest(&challenges) {
            self.cache(Some(CachedAuth::Digest {
                challenge: d.clone(),
                nc: 0,
            }));
            return Ok(());
        }
        if challenges.contains(&Challenge::Basic) {
            if self.https || self.allow_basic_over_http {
                self.cache(Some(CachedAuth::Basic));
                return Ok(());
            }
            return Err(ProviderError::AuthFailed(
                "the camera only offers Basic authentication over plain HTTP; use HTTPS or \
                 explicitly allow Basic over HTTP for this camera"
                    .into(),
            ));
        }
        Err(ProviderError::AuthFailed(
            "the camera requested an unsupported authentication scheme".into(),
        ))
    }

    async fn send(
        &self,
        method: &Method,
        path: &str,
        body: Option<&Value>,
        auth: Option<&str>,
    ) -> Result<reqwest::Response, ProviderError> {
        let mut req = self
            .http
            .request(method.clone(), format!("{}{path}", self.base));
        if let Some(b) = body {
            req = req.json(b);
        }
        if let Some(a) = auth {
            let mut v = HeaderValue::from_str(a)
                .map_err(|_| ProviderError::Internal("invalid authorization header".into()))?;
            v.set_sensitive(true);
            req = req.header(AUTHORIZATION, v);
        }
        req.send().await.map_err(|e| self.transport_error(&e))
    }

    fn transport_error(&self, e: &reqwest::Error) -> ProviderError {
        if e.is_timeout() {
            return ProviderError::Timeout(format!("no response from {}", self.host));
        }
        // Walk the source chain for a TLS cause without exposing URLs.
        let mut src: Option<&dyn std::error::Error> = Some(e);
        while let Some(s) = src {
            if let Some(tls) = s.downcast_ref::<rustls::Error>() {
                return ProviderError::Tls(tls.to_string());
            }
            let text = s.to_string();
            if text.contains("certificate") || text.contains("handshake") {
                return ProviderError::Tls(text);
            }
            src = s.source();
        }
        if e.is_connect() {
            return ProviderError::Unreachable(format!("cannot connect to {}", self.host));
        }
        ProviderError::Unreachable(format!("request to {} failed", self.host))
    }

    /// Perform a request with authentication negotiation.
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        access: Access,
    ) -> Result<Reply, ProviderError> {
        let start = Instant::now();
        // Preemptive auth when we already know the scheme and the call needs it.
        let pre = (access == Access::Authenticated && self.cached().is_some())
            .then(|| self.authorization(&method, path))
            .flatten();
        let mut resp = self
            .send(&method, path, body.as_ref(), pre.as_deref())
            .await?;

        if resp.status() == StatusCode::UNAUTHORIZED {
            if self.creds.is_none() {
                return Err(ProviderError::AuthFailed(
                    "the camera requires credentials, and none are configured".into(),
                ));
            }
            // New or stale challenge: adopt it and retry once.
            self.adopt_challenge(resp.headers())?;
            let auth = self.authorization(&method, path);
            resp = self
                .send(&method, path, body.as_ref(), auth.as_deref())
                .await?;
            if resp.status() == StatusCode::UNAUTHORIZED {
                self.cache(None);
                return Err(ProviderError::AuthFailed(
                    "the camera rejected the configured credentials".into(),
                ));
            }
        }

        let status = resp.status();
        let body = resp.text().await.map_err(|e| self.transport_error(&e))?;
        let elapsed = start.elapsed();
        match status {
            s if s.is_success() => Ok(Reply { body, elapsed }),
            StatusCode::FORBIDDEN => Err(ProviderError::AuthFailed(
                "the configured account lacks the privileges for this request".into(),
            )),
            StatusCode::NOT_FOUND => Err(ProviderError::unsupported(
                path.split('?').next().unwrap_or(path),
            )),
            StatusCode::TOO_MANY_REQUESTS => Err(ProviderError::RateLimited(format!(
                "{} is throttling requests",
                self.host
            ))),
            s => Err(ProviderError::Protocol(format!("unexpected HTTP {s}"))),
        }
    }

    /// POST a VAPIX JSON-RPC style request and return the parsed body.
    pub async fn post_json(
        &self,
        path: &str,
        body: Value,
        access: Access,
    ) -> Result<(Value, Duration), ProviderError> {
        let reply = self.request(Method::POST, path, Some(body), access).await?;
        let value: Value = serde_json::from_str(&reply.body)
            .map_err(|_| ProviderError::Protocol(format!("{path} returned invalid JSON")))?;
        Ok((value, reply.elapsed))
    }
}
