use std::convert::Infallible;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use tower::{Layer, Service};

/// Tower layer that patches error response bodies into a unified format.
///
/// Locals' `Error::CustomError` serialises as
/// `{"error":"<code>","description":"<msg>"}` (format B) — the code is
/// stuffed into `error` and there is **no standalone `code` field**.
/// The frontend reads `body.code`, so format B is invisible to it.
///
/// This middleware intercepts JSON error responses that lack a `code`
/// field and rewrites them to `CodedErrorResponse` shape:
/// `{"error":"<status text>","code":"<original error value>","description":"<msg>"}`.
///
/// It also rewrites 500 "internal_server_error" responses from the
/// catch-all branch to include a generic `common.internal` code.
#[derive(Clone, Default)]
pub struct ErrorHandlerLayer;

impl<S> Layer<S> for ErrorHandlerLayer {
    type Service = ErrorHandlerMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ErrorHandlerMiddleware { inner }
    }
}

#[derive(Clone)]
pub struct ErrorHandlerMiddleware<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for ErrorHandlerMiddleware<S>
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
        Box::pin(async move {
            let response = inner.call(req).await?;
            Ok(patch_response(response).await)
        })
    }
}

/// Inspect a response; if it's a JSON error body without a `code` field,
/// rewrite it to include one.
async fn patch_response(response: Response) -> Response {
    let status = response.status();

    // Only inspect 4xx / 5xx responses.
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }

    // Must be JSON.
    let is_json = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("application/json"));

    if !is_json {
        return response;
    }

    // Read body bytes — need to take ownership since `to_bytes` consumes Body.
    let (parts, body) = response.into_parts();
    let body_bytes = if let Ok(bytes) = axum::body::to_bytes(body, 4096).await {
        bytes
    } else {
        let fallback = Response::from_parts(parts, Body::from(Bytes::new()));
        return fallback;
    };
    let mut response = Response::from_parts(parts, Body::from(body_bytes.clone()));

    // Parse JSON.
    let mut value: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return response,
    };

    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => return response,
    };

    // Already has a `code` field — format A, nothing to do.
    if obj.contains_key("code") {
        return response;
    }

    // Extract error + description.
    let error_val = match obj.get("error").and_then(|v| v.as_str()) {
        Some(e) => e.to_string(),
        None => return response,
    };

    let description = obj
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Heuristic: if `error` field contains a dot, it's a code (format B).
    // Example: "authz.super_admin_required" → code = "authz.super_admin_required"
    // Otherwise it's a status label from loco defaults (format C) — synthesize a code.
    let (code, status_text) = if error_val.contains('.') {
        // Format B: Error::CustomError — error field is the code.
        (
            error_val,
            status.canonical_reason().unwrap_or("Error").to_string(),
        )
    } else {
        // Format C: loco default (e.g. "not_found", "unauthorized").
        let code = match status {
            StatusCode::NOT_FOUND => "common.not_found",
            StatusCode::UNAUTHORIZED => "common.unauthorized",
            StatusCode::FORBIDDEN => "common.forbidden",
            StatusCode::BAD_REQUEST => "common.bad_request",
            StatusCode::INTERNAL_SERVER_ERROR => "common.internal",
            _ => "common.error",
        };
        (code.to_string(), error_val)
    };

    // Rewrite to CodedErrorResponse format (format A).
    obj.insert("error".to_string(), serde_json::Value::String(status_text));
    obj.insert("code".to_string(), serde_json::Value::String(code));
    obj.insert(
        "description".to_string(),
        serde_json::Value::String(description),
    );

    let new_body = serde_json::to_string(&value).unwrap_or_default();

    *response.body_mut() = Body::from(new_body);
    response
}
