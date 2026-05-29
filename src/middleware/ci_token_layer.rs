//! CI token middleware — gates the `/api/ci/*` family with a static shared
//! secret read from the `KNOTA_FOLD_CI_TOKEN` environment variable.
//!
//! Why a separate layer instead of reusing `CasbinAuthzLayer`?
//!
//! - CI calls have no JWT, no tenant, no user identity — they're invoked by
//!   GitHub Actions / CLI scripts during build pipelines.
//! - Casbin authz would either need a fake "system" user seeded into the DB
//!   (audit trail noise) or a per-policy bypass (more attack surface than a
//!   single env-gated layer).
//! - This layer compares the header against the env value in **constant time**
//!   to avoid timing oracles, and refuses *all* requests when the env var is
//!   unset (fail-closed).
//!
//! Behaviour:
//!
//! - `X-CI-Token` header missing or mismatched → `401 {"error": "..."}`.
//! - `KNOTA_FOLD_CI_TOKEN` env var unset or empty → `503 {"error": "..."}`
//!   (the route is configured but the secret was never provisioned).
//! - Otherwise the request flows through to the inner service unchanged.

use std::{
    convert::Infallible,
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::future::BoxFuture;
use tower::{Layer, Service};

use crate::views::errors::CodedErrorResponse;

const CI_TOKEN_ENV_VAR: &str = "KNOTA_FOLD_CI_TOKEN";
const CI_TOKEN_HEADER: &str = "x-ci-token";

#[derive(Clone)]
pub struct CiTokenLayer {
    /// Pre-loaded once at construction so we don't hit the env on every
    /// request. `None` means the env var is unset → fail closed.
    expected: Arc<Option<String>>,
}

impl CiTokenLayer {
    pub fn new() -> Self {
        let expected = std::env::var(CI_TOKEN_ENV_VAR)
            .ok()
            .filter(|s| !s.trim().is_empty());
        if expected.is_none() {
            tracing::warn!(
                env_var = CI_TOKEN_ENV_VAR,
                "CI token env var is unset — all /api/ci/* requests will be rejected with 503"
            );
        }
        Self {
            expected: Arc::new(expected),
        }
    }
}

impl Default for CiTokenLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for CiTokenLayer {
    type Service = CiTokenMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CiTokenMiddleware {
            inner,
            expected: self.expected.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CiTokenMiddleware<S> {
    inner: S,
    expected: Arc<Option<String>>,
}

impl<S> Service<Request<Body>> for CiTokenMiddleware<S>
where
    S: Service<Request<Body>, Response = Response, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let expected = self.expected.clone();

        Box::pin(async move {
            let Some(expected_token) = expected.as_ref() else {
                return Ok(error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "ci.token_not_provisioned",
                    "CI token not provisioned on this server",
                ));
            };

            let presented = req
                .headers()
                .get(CI_TOKEN_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            // Constant-time compare to avoid leaking length / prefix info via timing.
            let presented_bytes = presented.as_bytes();
            let expected_bytes = expected_token.as_bytes();
            let ok = constant_time_eq(presented_bytes, expected_bytes);

            if !ok {
                tracing::warn!(
                    path = %req.uri().path(),
                    "CI token mismatch — rejecting request"
                );
                return Ok(error_response(
                    StatusCode::UNAUTHORIZED,
                    "ci.invalid_token",
                    "invalid CI token",
                ));
            }

            inner.call(req).await
        })
    }
}

fn error_response(status: StatusCode, code: &str, description: &str) -> Response {
    let body = CodedErrorResponse {
        error: status.canonical_reason().unwrap_or("Error").to_string(),
        code: Some(code.to_string()),
        description: description.to_string(),
    };
    (
        status,
        Json(serde_json::to_value(&body).unwrap_or_default()),
    )
        .into_response()
}

/// Length-then-byte constant-time comparison. Returns `false` immediately on
/// length mismatch (length itself is not secret — the env-side length is fixed).
/// For equal-length inputs, every byte is XOR'd into an accumulator so the
/// runtime does not depend on the position of the first differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn ct_eq_matches() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn ct_eq_rejects_diff_content() {
        assert!(!constant_time_eq(b"abc", b"abd"));
    }

    #[test]
    fn ct_eq_rejects_diff_length() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
