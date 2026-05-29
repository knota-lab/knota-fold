use crate::log_error;
use crate::utils::error::{IntoLocoResult, IntoModelResult};
use crate::views::errors::err_bad_request;
use crate::{
    extractors::tenant::TenantContext,
    mailers::auth::AuthMailer,
    models::{_entities::users, roles, users::RegisterParams},
    services::{auth_cache, captcha_service, login_guard},
    views::auth::{
        CaptchaResponse, ChangePasswordRequest, CurrentResponse, LoginErrorResponse,
        LoginRequest, LoginResponse, UnlockAccountRequest, UpdateProfileRequest,
    },
};
use axum::http::StatusCode;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use regex::Regex;
use sea_orm::ActiveValue;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub static EMAIL_DOMAIN_RE: OnceLock<Regex> = OnceLock::new();

fn get_allow_email_domain_re() -> &'static Regex {
    EMAIL_DOMAIN_RE.get_or_init(|| {
        Regex::new(r"@example\.com$|@gmail\.com$").expect("Failed to compile regex")
    })
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForgotParams {
    pub email: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResetParams {
    pub token: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MagicLinkParams {
    pub email: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResendVerificationParams {
    pub email: String,
}

/// Register function creates a new user with the given parameters and sends a
/// welcome email to the user
#[utoipa::path(
    post,
    path = "/api/auth/register",
    tag = "认证",
    description = "用户注册",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn register(
    State(ctx): State<AppContext>,
    Json(params): Json<RegisterParams>,
) -> Result<Response> {
    let res = users::Model::create_with_password(&ctx.db, &params).await;

    let user = match res {
        Ok(user) => user,
        Err(err) => {
            tracing::info!(
                message = err.to_string(),
                user_email = &params.email,
                "could not register user",
            );
            return format::json(());
        }
    };

    let user = user
        .into_active_model()
        .set_email_verification_sent(&ctx.db)
        .await?;

    AuthMailer::send_welcome(&ctx, &user).await?;

    format::json(())
}

/// Verify register user. if the user not verified his email, he can't login to
/// the system.
#[utoipa::path(
    get,
    path = "/api/auth/verify/{token}",
    tag = "认证",
    description = "邮箱验证",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn verify(
    State(ctx): State<AppContext>,
    Path(token): Path<String>,
) -> Result<Response> {
    let Ok(user) = users::Model::find_by_verification_token(&ctx.db, &token).await else {
        return unauthorized("invalid token");
    };

    if user.email_verified_at.is_some() {
        tracing::info!(user_id = user.id.to_string(), "user already verified");
    } else {
        let active_model = user.into_active_model();
        let user = active_model.verified(&ctx.db).await?;
        tracing::info!(user_id = user.id.to_string(), "user verified");
    }

    format::json(())
}

/// In case the user forgot his password  this endpoints generate a forgot token
/// and send email to the user. In case the email not found in our DB, we are
/// returning a valid request for for security reasons (not exposing users DB
/// list).
#[utoipa::path(
    post,
    path = "/api/auth/forgot",
    tag = "认证",
    description = "忘记密码",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn forgot(
    State(ctx): State<AppContext>,
    Json(params): Json<ForgotParams>,
) -> Result<Response> {
    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        return format::json(());
    };

    let user = user
        .into_active_model()
        .set_forgot_password_sent(&ctx.db)
        .await?;

    AuthMailer::forgot_password(&ctx, &user).await?;

    format::json(())
}

/// reset user password by the given parameters
#[utoipa::path(
    post,
    path = "/api/auth/reset",
    tag = "认证",
    description = "重置密码",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn reset(
    State(ctx): State<AppContext>,
    Json(params): Json<ResetParams>,
) -> Result<Response> {
    let Ok(user) = users::Model::find_by_reset_token(&ctx.db, &params.token).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        tracing::info!("reset token not found");

        return format::json(());
    };
    user.into_active_model()
        .reset_password(&ctx.db, &params.password)
        .await?;

    format::json(())
}

/// Issue a fresh stateless captcha. The client renders `image` and submits
/// `token` + the user-typed solution back via `POST /api/auth/login`.
#[utoipa::path(
    get,
    path = "/api/auth/captcha",
    tag = "认证",
    description = "获取登录验证码（无状态，token 内含答案与过期时间）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn captcha(State(ctx): State<AppContext>) -> Result<Response> {
    let (image, token) = captcha_service::generate(&ctx)?;
    // Mirror the configured TTL so the UI can auto-refresh; default 300s
    // when settings are unavailable.
    let ttl_seconds = ctx
        .config
        .settings
        .as_ref()
        .and_then(|v| v.get("captcha"))
        .and_then(|v| v.get("ttlSeconds"))
        .and_then(|v| v.as_u64())
        .unwrap_or(300);
    format::json(CaptchaResponse {
        image,
        token,
        ttl_seconds,
    })
}

/// Build a JSON error response with a specific HTTP status code.
fn login_error_with_status(
    status: StatusCode,
    body: LoginErrorResponse,
) -> Result<Response> {
    log_error!(&body.code, status.as_u16(), "login error");
    let payload = serde_json::to_value(&body).loco_err()?;
    Ok((status, axum::Json(payload)).into_response())
}

/// Record a failed login attempt and convert it into the right HTTP error.
/// Returns either a `423 ACCOUNT_LOCKED` (if the new failure crossed the
/// lock threshold) or the caller-supplied `fallback` error with the
/// `requireCaptcha` flag updated to reflect the new failure count.
async fn record_and_respond(
    ctx: &AppContext,
    email_lc: &str,
    thresholds: &login_guard::LoginThresholds,
    mut fallback: LoginErrorResponse,
) -> Result<Response> {
    let (_, require, lock_until) =
        login_guard::record_failure(ctx, email_lc, thresholds).await;
    if let Some(unlock_at) = lock_until {
        return login_error_with_status(
            StatusCode::LOCKED,
            LoginErrorResponse::account_locked(unlock_at),
        );
    }
    // Promote requireCaptcha if the new failure count crossed the captcha
    // threshold — the client uses this to render the captcha block on the
    // next attempt.
    if require {
        fallback.require_captcha = true;
    }
    let status = match fallback.code.as_str() {
        "CAPTCHA_REQUIRED" | "CAPTCHA_INVALID" => StatusCode::BAD_REQUEST,
        _ => StatusCode::UNAUTHORIZED,
    };
    login_error_with_status(status, fallback)
}

/// Creates a user login and returns a token
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "认证",
    description = "用户登录（含图形验证码与失败次数限制）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
#[tracing::instrument(skip_all)]
pub(crate) async fn login(
    State(ctx): State<AppContext>,
    Json(params): Json<LoginRequest>,
) -> Result<Response> {
    let email_lc = params.email.trim().to_lowercase();
    let thresholds = login_guard::load_thresholds(&ctx).await;

    tracing::info!(email = %email_lc, "LOGIN_ATTEMPT email={email_lc}");

    // 1. Lock gate (cheapest, before touching DB)
    match login_guard::pre_login_check(&ctx, &email_lc, &thresholds).await {
        login_guard::LoginGate::Locked { unlock_at_epoch } => {
            tracing::info!(email = %email_lc, "LOGIN_LOCKED email={email_lc}");
            return login_error_with_status(
                StatusCode::LOCKED,
                LoginErrorResponse::account_locked(unlock_at_epoch),
            );
        }
        login_guard::LoginGate::RequireCaptcha => {
            // Require captcha BEFORE password check so brute-force can't
            // bypass it by ignoring the flag. A missing OR wrong captcha is
            // treated as a failed login attempt and counted toward the lock
            // threshold — otherwise an attacker could spam wrong captchas
            // indefinitely without ever tripping the lock.
            let token = params.captcha_token.as_deref().unwrap_or("");
            let answer = params.captcha_answer.as_deref().unwrap_or("");
            if token.is_empty() || answer.is_empty() {
                return record_and_respond(
                    &ctx,
                    &email_lc,
                    &thresholds,
                    LoginErrorResponse::captcha_required(),
                )
                .await;
            }
            if !captcha_service::verify(&ctx, token, answer)? {
                return record_and_respond(
                    &ctx,
                    &email_lc,
                    &thresholds,
                    LoginErrorResponse::captcha_invalid(),
                )
                .await;
            }
        }
        login_guard::LoginGate::Allow => {
            // Optional captcha may still be present (UI sends it pre-emptively
            // once it has shown the field). If present and wrong we still
            // count it as a failure so wrong-captcha spam can't dodge the
            // lock counter even before the captcha gate engages.
            if let (Some(token), Some(answer)) = (
                params.captcha_token.as_deref(),
                params.captcha_answer.as_deref(),
            ) {
                if !token.is_empty()
                    && !answer.is_empty()
                    && !captcha_service::verify(&ctx, token, answer)?
                {
                    return record_and_respond(
                        &ctx,
                        &email_lc,
                        &thresholds,
                        LoginErrorResponse::captcha_invalid(),
                    )
                    .await;
                }
            }
        }
    }

    // 2. Credentials check
    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        tracing::info!(email = %email_lc, "LOGIN_USER_NOT_FOUND email={email_lc}");
        return record_and_respond(
            &ctx,
            &email_lc,
            &thresholds,
            LoginErrorResponse::invalid_credentials(false),
        )
        .await;
    };

    tracing::info!(email = %email_lc, user_id = %user.id, "LOGIN_USER_FOUND email={email_lc} user_id={}", user.id);

    let valid = user.verify_password(&params.password);
    tracing::info!(email = %email_lc, valid, "LOGIN_VERIFY_RESULT email={email_lc} valid={valid}");
    if !valid {
        return record_and_respond(
            &ctx,
            &email_lc,
            &thresholds,
            LoginErrorResponse::invalid_credentials(false),
        )
        .await;
    }

    if user.status == "disabled" {
        // Disabled is a permanent state, not a brute-force signal — do NOT
        // increment the failure counter, but DO clear any stale counter so
        // a subsequently re-enabled account starts clean.
        login_guard::record_success(&ctx, &email_lc).await;
        return login_error_with_status(
            StatusCode::UNAUTHORIZED,
            LoginErrorResponse::account_disabled(),
        );
    }

    let jwt_secret = ctx.config.get_jwt_config()?;

    let token = match user
        .generate_jwt(&ctx.db, &jwt_secret.secret, jwt_secret.expiration)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(email = %email_lc, error = %e, "LOGIN_JWT_GENERATION_FAILED email={email_lc} error={e}");
            return login_error_with_status(
                StatusCode::UNAUTHORIZED,
                LoginErrorResponse::invalid_credentials(false),
            );
        }
    };

    // 3. Success → clear counters
    login_guard::record_success(&ctx, &email_lc).await;

    format::json(LoginResponse::new(&user, &token))
}

#[utoipa::path(
    get,
    path = "/api/auth/current",
    tag = "认证",
    description = "获取当前用户",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn current(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let profile = auth_cache::get_user_profile(&ctx.cache, &ctx.db, tc.user_id).await?;
    let role_codes =
        roles::Model::find_user_role_codes(&ctx.db, tc.user_id, tc.tenant_id)
            .await
            .model_err()?;
    format::json(CurrentResponse::from_cached(
        &profile,
        tc.tenant_id,
        &tc.tenant_code,
        &tc.tenant_name,
        role_codes,
        tc.is_super_admin,
        tc.is_tenant_admin,
    ))
}

/// Magic link authentication provides a secure and passwordless way to log in to the application.
///
/// # Flow
/// 1. **Request a Magic Link**:
///    A registered user sends a POST request to `/magic-link` with their email.
///    If the email exists, a short-lived, one-time-use token is generated and sent to the user's email.
///    For security and to avoid exposing whether an email exists, the response always returns 200, even if the email is invalid.
///
/// 2. **Click the Magic Link**:
///    The user clicks the link (/magic-link/{token}), which validates the token and its expiration.
///    If valid, the server generates a JWT and responds with a [`LoginResponse`].
///    If invalid or expired, an unauthorized response is returned.
///
/// This flow enhances security by avoiding traditional passwords and providing a seamless login experience.
#[utoipa::path(
    post,
    path = "/api/auth/magic-link",
    tag = "认证",
    description = "发送Magic Link",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn magic_link(
    State(ctx): State<AppContext>,
    Json(params): Json<MagicLinkParams>,
) -> Result<Response> {
    let email_regex = get_allow_email_domain_re();
    if !email_regex.is_match(&params.email) {
        tracing::debug!(
            email = params.email,
            "The provided email is invalid or does not match the allowed domains"
        );
        return bad_request("invalid request");
    }

    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        tracing::debug!(email = params.email, "user not found by email");
        return format::empty_json();
    };

    let user = user.into_active_model().create_magic_link(&ctx.db).await?;
    AuthMailer::send_magic_link(&ctx, &user).await?;

    format::empty_json()
}

/// Verifies a magic link token and authenticates the user.
#[utoipa::path(
    get,
    path = "/api/auth/magic-link/{token}",
    tag = "认证",
    description = "验证Magic Link",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn magic_link_verify(
    Path(token): Path<String>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let Ok(user) = users::Model::find_by_magic_token(&ctx.db, &token).await else {
        // we don't want to expose our users email. if the email is invalid we still
        // returning success to the caller
        return unauthorized("unauthorized!");
    };

    if user.status == "disabled" {
        return unauthorized("Account is disabled");
    }

    let user = user.into_active_model().clear_magic_link(&ctx.db).await?;

    let jwt_secret = ctx.config.get_jwt_config()?;

    let token = user
        .generate_jwt(&ctx.db, &jwt_secret.secret, jwt_secret.expiration)
        .await
        .or_else(|_| unauthorized("unauthorized!"))?;

    format::json(LoginResponse::new(&user, &token))
}

#[utoipa::path(
    post,
    path = "/api/auth/resend-verification-mail",
    tag = "认证",
    description = "重新发送验证邮件",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn resend_verification_email(
    State(ctx): State<AppContext>,
    Json(params): Json<ResendVerificationParams>,
) -> Result<Response> {
    let Ok(user) = users::Model::find_by_email(&ctx.db, &params.email).await else {
        tracing::info!(
            email = params.email,
            "User not found for resend verification"
        );
        return format::json(());
    };

    if user.email_verified_at.is_some() {
        tracing::info!(
            user_id = user.id.to_string(),
            "User already verified, skipping resend"
        );
        return format::json(());
    }

    let user = user
        .into_active_model()
        .set_email_verification_sent(&ctx.db)
        .await?;

    AuthMailer::send_welcome(&ctx, &user).await?;
    tracing::info!(user_id = user.id.to_string(), "Verification email re-sent");

    format::json(())
}

/// Update the current user's profile (name and/or avatar).
#[utoipa::path(
    put,
    path = "/api/auth/profile",
    tag = "认证",
    description = "更新个人信息（姓名、头像）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update_profile(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<UpdateProfileRequest>,
) -> Result<Response> {
    let user = users::Model::find_by_id_str(&ctx.db, &tc.user_id.to_string()).await?;
    let mut active = user.into_active_model();

    if let Some(ref name) = params.name {
        let trimmed = name.trim();
        if trimmed.len() < 2 {
            return crate::views::errors::bad_request(
                "auth.name_too_short",
                "姓名至少 2 个字符",
            );
        }
        active.name = ActiveValue::Set(trimmed.to_string());
    }

    if let Some(ref avatar_id_str) = params.avatar_file_id {
        let avatar_uuid = uuid::Uuid::parse_str(avatar_id_str).map_err(|_| {
            loco_rs::Error::CustomError(
                axum::http::StatusCode::BAD_REQUEST,
                loco_rs::controller::ErrorDetail::new(
                    "auth.invalid_avatar_file_id",
                    "无效的 avatar_file_id",
                ),
            )
        })?;
        active.avatar_file_id = ActiveValue::Set(Some(avatar_uuid));
    }

    let updated = active.update(&ctx.db).await.model_err()?;

    // Invalidate cached profile so subsequent requests see updated data.
    auth_cache::invalidate_user(&ctx.cache, tc.user_id).await;

    let role_codes =
        roles::Model::find_user_role_codes(&ctx.db, tc.user_id, tc.tenant_id)
            .await
            .model_err()?;

    format::json(CurrentResponse::new(
        &updated,
        tc.tenant_id,
        &tc.tenant_code,
        &tc.tenant_name,
        role_codes,
        tc.is_super_admin,
        tc.is_tenant_admin,
    ))
}

/// Change the current user's password. Requires old password verification.
#[utoipa::path(
    post,
    path = "/api/auth/change-password",
    tag = "认证",
    description = "修改密码（需验证原密码）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn change_password(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<ChangePasswordRequest>,
) -> Result<Response> {
    let user = users::Model::find_by_id_str(&ctx.db, &tc.user_id.to_string()).await?;

    if !user.verify_password(&params.old_password) {
        return crate::views::errors::bad_request("auth.wrong_password", "原密码错误");
    }

    if params.new_password.len() < 6 {
        return crate::views::errors::bad_request(
            "auth.password_too_short",
            "新密码至少 6 个字符",
        );
    }

    if params.old_password == params.new_password {
        return crate::views::errors::bad_request(
            "auth.password_same_as_old",
            "新密码不能与原密码相同",
        );
    }

    user.into_active_model()
        .reset_password(&ctx.db, &params.new_password)
        .await?;

    // Invalidate cache so next auth check sees the new password_changed_at.
    auth_cache::invalidate_user(&ctx.cache, tc.user_id).await;

    format::json(())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/auth")
        .add("/register", openapi(post(register), routes!(register)))
        .add("/verify/{token}", openapi(get(verify), routes!(verify)))
        .add("/login", openapi(post(login), routes!(login)))
        .add("/captcha", openapi(get(captcha), routes!(captcha)))
        .add("/forgot", openapi(post(forgot), routes!(forgot)))
        .add("/reset", openapi(post(reset), routes!(reset)))
        .add("/current", openapi(get(current), routes!(current)))
        .add(
            "/profile",
            openapi(put(update_profile), routes!(update_profile)),
        )
        .add(
            "/change-password",
            openapi(post(change_password), routes!(change_password)),
        )
        .add(
            "/magic-link",
            openapi(post(magic_link), routes!(magic_link)),
        )
        .add(
            "/magic-link/{token}",
            openapi(get(magic_link_verify), routes!(magic_link_verify)),
        )
        .add(
            "/resend-verification-mail",
            openapi(
                post(resend_verification_email),
                routes!(resend_verification_email),
            ),
        )
}

/// Administratively clear the login lock + sliding-window failure counter
/// for a user identified by primary email. Idempotent — calling on a
/// not-locked account is a 200 no-op so the UI button can be enabled
/// without first re-checking lock state.
///
/// Permission: SUPER_ADMIN only. Tenant admins are intentionally NOT
/// allowed (per product decision) — locking is a security primitive that
/// platform operators own.
#[utoipa::path(
    post,
    path = "/api/admin/auth/unlock",
    tag = "认证",
    description = "[超管] 解锁因连续登录失败而被冻结的账号",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn unlock_account(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<UnlockAccountRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::forbidden(
            "auth.super_admin_required",
            "仅超级管理员可解锁账户",
        );
    }

    let email = params.email.trim();
    if email.is_empty() {
        return Err(err_bad_request("auth.email_required", "邮箱地址是必需的"));
    }
    // login_guard normalises to lowercase internally, so callers do not
    // need to pre-normalise. We pass the raw trimmed email through so
    // logging shows what the admin actually submitted.
    login_guard::unlock(&ctx, email).await;

    tracing::info!(
        admin_user_id = %tc.user_id,
        target_email = email,
        "admin unlocked account"
    );

    format::json(serde_json::json!({ "success": true }))
}

/// Super-admin-only auth routes. Mounted under `/api/admin/auth/*` so the
/// casbin layer can apply the SUPER_ADMIN policy via the URL prefix.
pub fn super_admin_routes() -> Routes {
    Routes::new().prefix("/api/admin/auth").add(
        "/unlock",
        openapi(post(unlock_account), routes!(unlock_account)),
    )
}
