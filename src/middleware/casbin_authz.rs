use std::{
    convert::Infallible,
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    extract::MatchedPath,
    http::{header, Request},
    response::{IntoResponse, Response},
};
use casbin::{CoreApi, Enforcer, MgmtApi, RbacApi};
use futures_util::future::BoxFuture;
use loco_rs::auth::jwt::UserClaims;
use loco_rs::cache;
use sea_orm::DatabaseConnection;
use tokio::sync::RwLock;
use tower::{Layer, Service};
use uuid::Uuid;

use crate::models::{roles, tenants};
use crate::services::{api_key_service::ApiKeyIdentity, auth_cache};
use crate::views::errors::{err_forbidden, err_internal, err_unauthorized};

const WHITELIST_PATHS: &[&str] = &["/api/auth/current", "/api/users/me/menus"];

const SUPER_ADMIN_ROLE: &str = "SUPER_ADMIN";

// ── Shared context to avoid passing individual fields everywhere ───────────

struct AuthzCtx {
    enforcer: Arc<RwLock<Enforcer>>,
    db: DatabaseConnection,
    cache: Arc<cache::Cache>,
}

// ── Layer / Middleware structs (unchanged) ──────────────────────────────────

#[derive(Clone)]
pub struct CasbinAuthzLayer {
    enforcer: Arc<RwLock<Enforcer>>,
    db: DatabaseConnection,
    jwt_secret: String,
    cache: Arc<cache::Cache>,
}

impl CasbinAuthzLayer {
    pub const fn new(
        enforcer: Arc<RwLock<Enforcer>>,
        db: DatabaseConnection,
        jwt_secret: String,
        cache: Arc<cache::Cache>,
    ) -> Self {
        Self {
            enforcer,
            db,
            jwt_secret,
            cache,
        }
    }
}

impl<S> Layer<S> for CasbinAuthzLayer {
    type Service = CasbinAuthzMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CasbinAuthzMiddleware {
            inner,
            enforcer: self.enforcer.clone(),
            db: self.db.clone(),
            jwt_secret: self.jwt_secret.clone(),
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CasbinAuthzMiddleware<S> {
    inner: S,
    enforcer: Arc<RwLock<Enforcer>>,
    db: DatabaseConnection,
    jwt_secret: String,
    cache: Arc<cache::Cache>,
}

// ── Service impl: call only dispatches ─────────────────────────────────────

impl<S> Service<Request<Body>> for CasbinAuthzMiddleware<S>
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
        let ctx = AuthzCtx {
            enforcer: self.enforcer.clone(),
            db: self.db.clone(),
            cache: self.cache.clone(),
        };
        let jwt_secret = self.jwt_secret.clone();

        Box::pin(async move {
            let matched_path = extract_matched_path(&req);

            if WHITELIST_PATHS.contains(&matched_path.as_str()) {
                return inner.call(req).await;
            }

            let method = req.method().as_str().to_uppercase();

            let Some(token) = extract_bearer_token(&req) else {
                return Ok(
                    err_unauthorized("authz.no_token", "未提供认证令牌").into_response()
                );
            };

            // Try JWT first, fall back to API Key
            if let Ok(claims) = loco_rs::auth::jwt::JWT::new(&jwt_secret).validate(&token)
            {
                return handle_jwt_authz(
                    claims.claims,
                    req,
                    &mut inner,
                    &ctx,
                    &matched_path,
                    &method,
                )
                .await;
            }

            handle_api_key_authz(token, req, &mut inner, &ctx, &matched_path, &method)
                .await
        })
    }
}

// ── JWT authentication + authorization ─────────────────────────────────────

async fn handle_jwt_authz<S>(
    claims: UserClaims,
    req: Request<Body>,
    inner: &mut S,
    ctx: &AuthzCtx,
    matched_path: &str,
    method: &str,
) -> Result<Response, Infallible>
where
    S: Service<Request<Body>, Response = Response, Error = Infallible> + Send,
    S::Future: Send,
{
    let Ok(user_id) = Uuid::parse_str(&claims.pid) else {
        return Ok(
            err_unauthorized("authz.invalid_token", "认证令牌无效").into_response()
        );
    };

    if let Err(resp) = verify_password_freshness(&claims, ctx, user_id).await {
        return Ok(resp);
    }

    if let Err(resp) = verify_user_active(ctx, user_id).await {
        return Ok(resp);
    }

    let tenant_id = match resolve_tenant(&claims, &ctx.db).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let role_codes =
        match roles::Model::find_user_role_codes(&ctx.db, user_id, tenant_id).await {
            Ok(codes) => codes,
            Err(err) => {
                tracing::error!(
                    error = %err,
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    "failed to load user roles for authorization"
                );
                return Ok(err_internal("authz.roles_load_failed", "用户角色加载失败")
                    .into_response());
            }
        };

    if role_codes.iter().any(|code| code == SUPER_ADMIN_ROLE) {
        return inner.call(req).await;
    }

    let user_id_str = user_id.to_string();
    let tenant_id_str = tenant_id.to_string();

    if !casbin_enforce(
        &ctx.enforcer,
        &user_id_str,
        &tenant_id_str,
        matched_path,
        method,
    )
    .await
    {
        return Ok(err_forbidden(
            "authz.access_denied",
            format!("无权访问 {method} {matched_path}，请联系管理员分配对应权限"),
        )
        .into_response());
    }

    inner.call(req).await
}

// ── API Key authentication + authorization ─────────────────────────────────

async fn handle_api_key_authz<S>(
    token: String,
    mut req: Request<Body>,
    inner: &mut S,
    ctx: &AuthzCtx,
    matched_path: &str,
    method: &str,
) -> Result<Response, Infallible>
where
    S: Service<Request<Body>, Response = Response, Error = Infallible> + Send,
    S::Future: Send,
{
    let Ok(api_key_identity) = ApiKeyIdentity::authenticate(&ctx.db, &token).await else {
        return Ok(
            err_unauthorized("authz.api_key_invalid", "API Key 认证失败").into_response(),
        );
    };

    let subject = format!("apikey:{}", api_key_identity.api_key_id);
    let tenant_id_str = api_key_identity.tenant_id.to_string();

    if !casbin_enforce(
        &ctx.enforcer,
        &subject,
        &tenant_id_str,
        matched_path,
        method,
    )
    .await
    {
        return Ok(err_forbidden(
            "authz.api_key_access_denied",
            format!("API Key 无权访问 {method} {matched_path}"),
        )
        .into_response());
    }

    tracing::Span::current().record(
        "api_key_id",
        tracing::field::display(api_key_identity.api_key_id),
    );
    tracing::Span::current().record("auth_type", tracing::field::display("api_key"));

    req.extensions_mut().insert(api_key_identity);
    inner.call(req).await
}

// ── Fine-grained helpers ───────────────────────────────────────────────────

/// Verify token `password_iat` is not older than DB record.
async fn verify_password_freshness(
    claims: &UserClaims,
    ctx: &AuthzCtx,
    user_id: Uuid,
) -> Result<(), Response> {
    let token_pwd_iat = claims
        .claims
        .get("password_iat")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    let db_pwd_iat = auth_cache::get_password_iat(&ctx.cache, &ctx.db, user_id)
        .await
        .map_err(|_| {
            err_unauthorized("authz.token_validation_failed", "令牌验证失败")
                .into_response()
        })?;

    if token_pwd_iat < db_pwd_iat {
        return Err(
            err_unauthorized("authz.password_changed", "密码已修改，请重新登录")
                .into_response(),
        );
    }

    Ok(())
}

/// Verify user account is not disabled.
async fn verify_user_active(ctx: &AuthzCtx, user_id: Uuid) -> Result<(), Response> {
    let profile = auth_cache::get_user_profile(&ctx.cache, &ctx.db, user_id)
        .await
        .map_err(|_| {
            err_unauthorized("authz.user_load_failed", "用户信息加载失败").into_response()
        })?;

    if profile.status == "disabled" {
        return Err(
            err_forbidden("authz.account_disabled", "账号已被禁用").into_response()
        );
    }

    Ok(())
}

/// Parse `tenant_code` from claims and look up `tenant_id`.
async fn resolve_tenant(
    claims: &UserClaims,
    db: &DatabaseConnection,
) -> Result<Uuid, Response> {
    let tenant_code = claims
        .claims
        .get("tenant_code")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            err_unauthorized("authz.no_tenant_in_token", "令牌中缺少租户信息")
                .into_response()
        })?
        .to_string();

    let tenant = tenants::Model::find_by_code(db, &tenant_code)
        .await
        .map_err(|_| {
            err_unauthorized("authz.tenant_not_found", "令牌中租户不存在").into_response()
        })?;

    Ok(tenant.id)
}

/// Call Casbin enforcer and emit debug/warn logs.
async fn casbin_enforce(
    enforcer: &Arc<RwLock<Enforcer>>,
    subject: &str,
    tenant_id: &str,
    path: &str,
    method: &str,
) -> bool {
    let enforcer = enforcer.read().await;

    let roles_for_user = enforcer.get_roles_for_user(subject, Some(tenant_id));
    let policies_for_roles: Vec<Vec<String>> = roles_for_user
        .iter()
        .flat_map(|role: &String| enforcer.get_filtered_policy(0, vec![role.clone()]))
        .collect();

    tracing::debug!(
        subject = %subject,
        tenant_id = %tenant_id,
        path = %path,
        method = %method,
        roles = ?roles_for_user,
        policy_count = policies_for_roles.len(),
        "casbin enforce input"
    );

    let result = enforcer
        .enforce((subject, tenant_id, path, method))
        .unwrap_or(false);

    if !result {
        tracing::warn!(
            subject = %subject,
            tenant_id = %tenant_id,
            path = %path,
            method = %method,
            roles = ?roles_for_user,
            policies = ?policies_for_roles,
            "casbin DENIED access"
        );
    }

    drop(enforcer);
    result
}

// ── Path / token extraction utilities ──────────────────────────────────────

fn extract_matched_path(req: &Request<Body>) -> String {
    let raw = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| req.uri().path(), MatchedPath::as_str)
        .to_string();

    if raw.len() > 1 && raw.ends_with('/') {
        raw[..raw.len() - 1].to_string()
    } else {
        raw
    }
}

fn extract_bearer_token(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string)
}
