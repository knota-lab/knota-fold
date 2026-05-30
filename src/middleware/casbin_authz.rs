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

#[derive(Clone)]
pub struct CasbinAuthzLayer {
    enforcer: Arc<RwLock<Enforcer>>,
    db: DatabaseConnection,
    jwt_secret: String,
    cache: Arc<cache::Cache>,
}

impl CasbinAuthzLayer {
    pub fn new(
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
        let enforcer = self.enforcer.clone();
        let db = self.db.clone();
        let jwt_secret = self.jwt_secret.clone();
        let cache = self.cache.clone();

        Box::pin(async move {
            let raw_path = req
                .extensions()
                .get::<MatchedPath>()
                .map_or_else(|| req.uri().path(), MatchedPath::as_str)
                .to_string();
            let matched_path = if raw_path.len() > 1 && raw_path.ends_with('/') {
                raw_path[..raw_path.len() - 1].to_string()
            } else {
                raw_path
            };

            if WHITELIST_PATHS.contains(&matched_path.as_str()) {
                return inner.call(req).await;
            }

            let method = req.method().as_str().to_uppercase();

            let Some(token) = extract_bearer_token(&req) else {
                return Ok(
                    err_unauthorized("authz.no_token", "未提供认证令牌").into_response()
                );
            };

            if let Ok(claims) = loco_rs::auth::jwt::JWT::new(&jwt_secret).validate(&token)
            {
                let Ok(user_id) = Uuid::parse_str(&claims.claims.pid) else {
                    return Ok(err_unauthorized("authz.invalid_token", "认证令牌无效")
                        .into_response());
                };

                let token_pwd_iat = claims
                    .claims
                    .claims
                    .get("password_iat")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0);

                let Ok(db_pwd_iat) =
                    auth_cache::get_password_iat(&cache, &db, user_id).await
                else {
                    return Ok(err_unauthorized(
                        "authz.token_validation_failed",
                        "令牌验证失败",
                    )
                    .into_response());
                };

                if token_pwd_iat < db_pwd_iat {
                    return Ok(err_unauthorized(
                        "authz.password_changed",
                        "密码已修改，请重新登录",
                    )
                    .into_response());
                }

                let Ok(user_profile) =
                    auth_cache::get_user_profile(&cache, &db, user_id).await
                else {
                    return Ok(err_unauthorized(
                        "authz.user_load_failed",
                        "用户信息加载失败",
                    )
                    .into_response());
                };
                if user_profile.status == "disabled" {
                    return Ok(err_forbidden("authz.account_disabled", "账号已被禁用")
                        .into_response());
                }

                let tenant_code = match claims
                    .claims
                    .claims
                    .get("tenant_code")
                    .and_then(|value| value.as_str())
                {
                    Some(tenant_code) => tenant_code.to_string(),
                    None => {
                        return Ok(err_unauthorized(
                            "authz.no_tenant_in_token",
                            "令牌中缺少租户信息",
                        )
                        .into_response())
                    }
                };

                let Ok(tenant) = tenants::Model::find_by_code(&db, &tenant_code).await
                else {
                    return Ok(err_unauthorized(
                        "authz.tenant_not_found",
                        "令牌中租户不存在",
                    )
                    .into_response());
                };

                let tenant_id = tenant.id;

                let role_codes = match roles::Model::find_user_role_codes(
                    &db, user_id, tenant_id,
                )
                .await
                {
                    Ok(role_codes) => role_codes,
                    Err(err) => {
                        tracing::error!(error = %err, user_id = %user_id, tenant_id = %tenant_id, "failed to load user roles for authorization");
                        return Ok(err_internal(
                            "authz.roles_load_failed",
                            "用户角色加载失败",
                        )
                        .into_response());
                    }
                };

                let is_super_admin =
                    role_codes.iter().any(|code| code == SUPER_ADMIN_ROLE);
                if is_super_admin {
                    return inner.call(req).await;
                }

                let user_id_str = user_id.to_string();
                let tenant_id_str = tenant_id.to_string();

                let allowed = {
                    let enforcer = enforcer.read().await;

                    let roles_for_user =
                        enforcer.get_roles_for_user(&user_id_str, Some(&tenant_id_str));
                    let policies_for_roles: Vec<Vec<String>> = roles_for_user
                        .iter()
                        .flat_map(|role: &String| {
                            enforcer.get_filtered_policy(0, vec![role.clone()])
                        })
                        .collect();

                    tracing::debug!(
                        user_id = %user_id_str,
                        tenant_id = %tenant_id_str,
                        path = %matched_path,
                        method = %method,
                        roles = ?roles_for_user,
                        policy_count = policies_for_roles.len(),
                        "casbin enforce input"
                    );

                    let result = enforcer
                        .enforce((
                            user_id_str.as_str(),
                            tenant_id_str.as_str(),
                            matched_path.as_str(),
                            method.as_str(),
                        ))
                        .unwrap_or(false);

                    if !result {
                        tracing::warn!(
                            user_id = %user_id_str,
                            tenant_id = %tenant_id_str,
                            path = %matched_path,
                            method = %method,
                            roles = ?roles_for_user,
                            policies = ?policies_for_roles,
                            "casbin DENIED access"
                        );
                    }

                    result
                };

                if !allowed {
                    return Ok(err_forbidden(
                        "authz.access_denied",
                        format!(
                            "无权访问 {method} {matched_path}，请联系管理员分配对应权限"
                        ),
                    )
                    .into_response());
                }

                return inner.call(req).await;
            }

            let Ok(api_key_identity) = ApiKeyIdentity::authenticate(&db, &token).await
            else {
                return Ok(
                    err_unauthorized("authz.api_key_invalid", "API Key 认证失败")
                        .into_response(),
                );
            };

            let subject = format!("apikey:{}", api_key_identity.api_key_id);
            let tenant_id_str = api_key_identity.tenant_id.to_string();

            let allowed = {
                let enforcer = enforcer.read().await;
                enforcer
                    .enforce((
                        subject.as_str(),
                        tenant_id_str.as_str(),
                        matched_path.as_str(),
                        method.as_str(),
                    ))
                    .unwrap_or(false)
            };

            if !allowed {
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
            tracing::Span::current()
                .record("auth_type", tracing::field::display("api_key"));

            let mut req = req;
            req.extensions_mut().insert(api_key_identity);

            inner.call(req).await
        })
    }
}

fn extract_bearer_token(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string)
}
