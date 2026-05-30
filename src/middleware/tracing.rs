use std::{
    convert::Infallible,
    net::SocketAddr,
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderValue, Request},
    response::Response,
};
use futures_util::future::BoxFuture;
use tower::{Layer, Service};
use tracing::Instrument;
use uuid::Uuid;

/// Newtype for trace_id stored in request extensions.
///
/// Set by [`TracingLayer`] before controllers run.
/// Read by [`crate::extractors::request_meta::RequestMeta`].
#[derive(Clone, Debug)]
pub struct TraceId(pub String);

/// Newtype for request_id stored in request extensions.
///
/// Set by [`TracingLayer`] before controllers run.
/// Read by [`crate::extractors::request_meta::RequestMeta`].
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Extract user_id (pid) and tenant_code from a JWT token without signature
/// verification. For logging purposes only — security enforcement happens in
/// CasbinAuthzLayer.
fn extract_jwt_claims(token: &str) -> Option<(String, String)> {
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    // JWT payload is URL-safe base64. Pad if needed.
    let input = parts[1];
    let padded = if input.len().is_multiple_of(4) {
        input.to_string()
    } else {
        format!("{input}{}", "=".repeat(4 - input.len() % 4))
    };
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(padded))
        .ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;

    let pid = payload.get("pid")?.as_str()?.to_string();
    let tenant_code = payload
        .get("tenant_code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((pid, tenant_code))
}

/// Extract client IP from request headers, falling back to socket address.
///
/// Priority:
/// 1. `X-Forwarded-For` first entry (reverse proxy)
/// 2. `X-Real-IP` (nginx/proxy_protocol)
/// 3. `ConnectInfo<SocketAddr>` (direct connection)
fn extract_ip(req: &Request<Body>) -> Option<String> {
    // Try header-based extraction first (behind reverse proxy).
    if let Some(ip) = req
        .headers()
        .get("x-forwarded-for")
        .or_else(|| req.headers().get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
    {
        return Some(ip);
    }

    // Fallback: direct connection socket address.
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
}

/// Tower layer that injects distributed tracing context into every request.
///
/// Responsibilities:
/// - Reads `X-Trace-Id` header (validated) or generates a fresh UUIDv4
/// - Generates a new UUIDv7 as `request_id`
/// - Stores both as typed extensions on the request
/// - Creates the root `http.request` tracing span with `trace_id` and `request_id` fields
/// - Echoes `X-Trace-Id` back in the response header
///
/// Must be mounted as the **outermost** layer (via `after_routes`) so it wraps
/// Casbin auth and all controllers.
#[derive(Clone, Default)]
pub struct TracingLayer;

impl<S> Layer<S> for TracingLayer {
    type Service = TracingMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TracingMiddleware { inner }
    }
}

#[derive(Clone)]
pub struct TracingMiddleware<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for TracingMiddleware<S>
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

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let start = Instant::now();
        let method = req.method().clone().to_string();
        let path = req.uri().path().to_string();

        // Extract IP before request is consumed.
        let ip_address = extract_ip(&req);

        // Extract user_id + tenant_code from JWT (no signature verification).
        let (user_id, tenant_code) = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .and_then(extract_jwt_claims)
            .map_or((None, None), |(pid, code)| (Some(pid), Some(code)));

        Box::pin(async move {
            // Extract incoming X-Trace-Id or generate UUIDv4.
            // Validation: non-empty, ≤128 chars, no control characters.
            let trace_id = req
                .headers()
                .get("x-trace-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| {
                    !s.is_empty() && s.len() <= 128 && s.chars().all(|c| !c.is_control())
                })
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            // Always generate a fresh UUIDv7 per request.
            let request_id = Uuid::now_v7().to_string();

            // Inject into extensions for downstream extractors.
            req.extensions_mut().insert(TraceId(trace_id.clone()));
            req.extensions_mut().insert(RequestId(request_id.clone()));

            // Root span — all downstream spans are children of this.
            let span = tracing::info_span!(
                "http.request",
                trace_id = %trace_id,
                request_id = %request_id,
                method = %method,
                path = %path,
                api_key_id = tracing::field::Empty,
                auth_type = tracing::field::Empty,
            );

            let result = inner.call(req).instrument(span).await;

            match result {
                Ok(response) => {
                    // Send request summary to app-logs (if module is enabled).
                    if let Some(sender) = crate::app_logs::layer::get_sender() {
                        let duration_ms = start.elapsed().as_millis() as u64;
                        let status = response.status().as_u16();

                        let error = if status >= 400 {
                            Some(format!("HTTP {status}"))
                        } else {
                            None
                        };

                        crate::app_logs::layer::try_send_log(
                            sender,
                            crate::app_logs::layer::LogEntry::request_summary(
                                &trace_id,
                                &request_id,
                                &method,
                                &path,
                                None::<&str>, // route: MatchedPath not available at this layer
                                status,
                                duration_ms,
                                user_id.as_deref(),
                                tenant_code.as_deref(),
                                ip_address.as_deref(),
                                error.as_deref(),
                            ),
                        );
                    }

                    // Echo trace_id so clients can correlate logs.
                    let mut response = response;
                    if let Ok(header_value) = HeaderValue::from_str(&trace_id) {
                        response.headers_mut().insert("x-trace-id", header_value);
                    }
                    Ok(response)
                }
                Err(e) => Err(e),
            }
        })
    }
}
