//! Uniform error response shape with a machine-readable `code` field.
//!
//! All user-facing API errors should return a JSON body of the form:
//! ```json
//! { "error": "Bad Request", "code": "auth.invalid_credentials", "description": "邮箱或密码错误" }
//! ```
//!
//! The `code` field is a stable dot-separated identifier that the frontend
//! errorMap can translate without scraping the human-readable message.
//!
//! # Convention
//!
//! - Code format: `<module>.<detail>` — e.g. `auth.invalid_credentials`,
//!   `i18n.namespace_invalid`, `files.quota_exceeded`.
//! - Keep codes stable across releases; the frontend `errorMap` maps them.
//! - When in doubt, omit `code` and the frontend falls back to displaying
//!   `description` as-is (which is always Chinese by convention).
//!
//! # Domain modules
//!
//! Prefer the typed domain helpers (`authz::super_admin_required()`,
//! `role::cross_tenant(...)`, etc.) over raw `forbidden(...)` calls.
//! They centralise codes and descriptions so the frontend errorMap
//! stays in sync without grep-ing the entire codebase.
//!
//! # New-style error construction (v2)
//!
//! Use `from_info()` / `from_info_with_desc()` + `ErrorInfo` constants for
//! compile-time type safety and automatic call-site location tracking.

use crate::log_error;
use crate::utils::error::IntoLocoResult;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::*;
use serde::Serialize;
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// Response body
// ---------------------------------------------------------------------------

/// Canonical error body with an optional machine-readable `code`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodedErrorResponse {
    /// HTTP status text (e.g. "Bad Request"). Kept for backward compat with
    /// loco's default shape `{"error": "..."}`.
    pub error: String,
    /// Stable machine code. When `None`, the serializer omits the field so
    /// existing clients that only read `error`/`description` are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable description (zh-CN by convention).
    pub description: String,
}

// ---------------------------------------------------------------------------
// New-style: from_info / from_info_with_desc (v2 core)
// ---------------------------------------------------------------------------

/// Construct a loco `Error` from `ErrorInfo`, automatically recording the
/// call-site location via `#[track_caller]`.
///
/// This is the **core construction entry point** — status / code / description
/// all come from the `ErrorInfo` enum, so the caller cannot mismatch them.
#[track_caller]
pub fn from_info(info: crate::error_info::ErrorInfo) -> Error {
    log_error!(info.code(), info.status().as_u16(), "error constructed");
    Error::CustomError(
        info.status(),
        ErrorDetail::new(info.code(), info.description()),
    )
}

/// Construct a loco `Error` from `ErrorInfo`, overriding the default
/// description with a dynamic string.
#[track_caller]
pub fn from_info_with_desc(
    info: crate::error_info::ErrorInfo,
    desc: impl Into<String>,
) -> Error {
    log_error!(
        info.code(),
        info.status().as_u16(),
        "error constructed with custom description"
    );
    Error::CustomError(info.status(), ErrorDetail::new(info.code(), &desc.into()))
}

/// Build a `CustomError` from raw parts. Replaces the 9 identical `fn custom_error`
/// copies that were scattered across controllers and services.
#[track_caller]
pub fn err_custom(status: StatusCode, code: &str, description: &str) -> Error {
    log_error!(code, status.as_u16(), "custom error");
    Error::CustomError(status, ErrorDetail::new(code, description))
}

/// Parse a string as UUID, returning 400 Bad Request on failure.
#[track_caller]
pub fn parse_uuid(id: impl AsRef<str>) -> Result<uuid::Uuid> {
    id.as_ref().parse::<uuid::Uuid>().map_or_else(
        |_| Err(from_info(crate::error_info::common::INVALID_UUID)),
        Ok,
    )
}

// ---------------------------------------------------------------------------
// Raw helpers — return `Result<Response>` (controllers)
// ---------------------------------------------------------------------------

/// Build a `CodedErrorResponse` and return it as a loco `Result<Response>`.
pub fn coded_error(
    status: StatusCode,
    code: impl Into<String>,
    description: impl Into<String>,
) -> Result<Response> {
    let body = CodedErrorResponse {
        error: status.canonical_reason().unwrap_or("Error").to_string(),
        code: Some(code.into()),
        description: description.into(),
    };
    let payload = serde_json::to_value(&body).loco_err()?;
    Ok((status, axum::Json(payload)).into_response())
}

/// Shorthand: 400 Bad Request with code.
pub fn bad_request(code: impl Into<String>, msg: impl Into<String>) -> Result<Response> {
    coded_error(StatusCode::BAD_REQUEST, code, msg)
}

/// Shorthand: 401 Unauthorized with code.
pub fn unauthorized(code: impl Into<String>, msg: impl Into<String>) -> Result<Response> {
    coded_error(StatusCode::UNAUTHORIZED, code, msg)
}

/// Shorthand: 403 Forbidden with code.
pub fn forbidden(code: impl Into<String>, msg: impl Into<String>) -> Result<Response> {
    coded_error(StatusCode::FORBIDDEN, code, msg)
}

/// Shorthand: 404 Not Found with code.
pub fn not_found(code: impl Into<String>, msg: impl Into<String>) -> Result<Response> {
    coded_error(StatusCode::NOT_FOUND, code, msg)
}

/// Shorthand: 409 Conflict with code.
pub fn conflict(code: impl Into<String>, msg: impl Into<String>) -> Result<Response> {
    coded_error(StatusCode::CONFLICT, code, msg)
}

/// Shorthand: 500 Internal Server Error with code.
pub fn internal(code: impl Into<String>, msg: impl Into<String>) -> Result<Response> {
    coded_error(StatusCode::INTERNAL_SERVER_ERROR, code, msg)
}

// ---------------------------------------------------------------------------
// Raw helpers — return `loco_rs::Error` (services / middleware)
// ---------------------------------------------------------------------------

/// Service-layer 403 error that preserves code + description through the
/// Loco error chain (unlike `Error::Unauthorized` which silently drops them).
#[track_caller]
pub fn err_forbidden(code: &str, msg: impl AsRef<str>) -> Error {
    log_error!(code, 403, "err_forbidden");
    Error::CustomError(StatusCode::FORBIDDEN, ErrorDetail::new(code, msg.as_ref()))
}

/// Service-layer 401 error with coded detail.
#[track_caller]
pub fn err_unauthorized(code: &str, msg: impl AsRef<str>) -> Error {
    log_error!(code, 401, "err_unauthorized");
    Error::CustomError(
        StatusCode::UNAUTHORIZED,
        ErrorDetail::new(code, msg.as_ref()),
    )
}

/// Service-layer 400 error with coded detail.
#[track_caller]
pub fn err_bad_request(code: &str, msg: impl AsRef<str>) -> Error {
    log_error!(code, 400, "err_bad_request");
    Error::CustomError(
        StatusCode::BAD_REQUEST,
        ErrorDetail::new(code, msg.as_ref()),
    )
}

/// Service-layer 404 error with coded detail.
#[track_caller]
pub fn err_not_found(code: &str, msg: impl AsRef<str>) -> Error {
    log_error!(code, 404, "err_not_found");
    Error::CustomError(StatusCode::NOT_FOUND, ErrorDetail::new(code, msg.as_ref()))
}

/// Service-layer 409 error with coded detail.
#[track_caller]
pub fn err_conflict(code: &str, msg: impl AsRef<str>) -> Error {
    log_error!(code, 409, "err_conflict");
    Error::CustomError(StatusCode::CONFLICT, ErrorDetail::new(code, msg.as_ref()))
}

/// Service-layer 500 error with coded detail.
#[track_caller]
pub fn err_internal(code: &str, msg: impl AsRef<str>) -> Error {
    log_error!(code, 500, "err_internal");
    Error::CustomError(
        StatusCode::INTERNAL_SERVER_ERROR,
        ErrorDetail::new(code, msg.as_ref()),
    )
}

// ---------------------------------------------------------------------------
// Domain-specific error constructors
// ---------------------------------------------------------------------------

/// Authorization (authz) errors.
pub mod authz {
    use super::{err_forbidden, forbidden, Error, Response, Result};

    /// 403 — 仅超级管理员可操作（controller 层，返回 Response）
    pub fn super_admin_required() -> Result<Response> {
        forbidden("authz.super_admin_required", "仅超级管理员可操作")
    }

    /// 403 — 仅超级管理员可操作（service/helper 层，返回 Error）
    #[must_use]
    pub fn err_super_admin_required() -> Error {
        err_forbidden("authz.super_admin_required", "仅超级管理员可操作")
    }

    /// 403 — 仅管理员可操作（controller 层）
    pub fn admin_required() -> Result<Response> {
        forbidden("authz.admin_required", "仅管理员可操作")
    }
}

/// Role-related errors.
pub mod role {
    use super::{err_forbidden, forbidden, Error, Response, Result};

    /// 403 — 跨租户操作角色
    pub fn cross_tenant(action: &str) -> Result<Response> {
        forbidden("role.cross_tenant", format!("不能跨租户{action}"))
    }

    /// 403 — 分配超出自身范围的权限（service 层）
    #[must_use]
    pub fn err_out_of_scope_permissions() -> Error {
        err_forbidden(
            "role.out_of_scope_permissions",
            "不能分配超出自身范围的权限",
        )
    }

    /// 403 — 分配超出自身范围的菜单（service 层）
    #[must_use]
    pub fn err_out_of_scope_menus() -> Error {
        err_forbidden("role.out_of_scope_menus", "不能分配超出自身范围的菜单")
    }
}

/// User-related errors.
pub mod user {
    use super::{forbidden, Response, Result};

    /// 403 — 跨租户操作用户
    pub fn cross_tenant(action: &str) -> Result<Response> {
        forbidden("user.cross_tenant", format!("不能跨租户{action}"))
    }
}

/// Worker-related errors.
pub mod worker {
    use super::{forbidden, Response, Result};

    /// 403 — 无权操作此 Worker
    pub fn not_authorized() -> Result<Response> {
        forbidden("worker.not_authorized", "无权操作此 Worker")
    }

    /// 403 — 无权操作他人的定时任务
    pub fn schedule_not_yours() -> Result<Response> {
        forbidden("worker_schedule.not_yours", "无权操作他人的定时任务")
    }

    /// 403 — 无权查看他人的执行记录
    pub fn execution_not_yours() -> Result<Response> {
        forbidden("worker_execution.not_yours", "无权查看他人的执行记录")
    }
}

/// Dictionary-related errors (service 层).
pub mod dict {
    use super::{err_forbidden, Error};

    /// 403 — 无权操作此字典类型
    #[must_use]
    pub fn err_type_forbidden() -> Error {
        err_forbidden("dict.type_forbidden", "无权操作此字典类型")
    }

    /// 403 — 无权操作此字典项
    #[must_use]
    pub fn err_item_forbidden() -> Error {
        err_forbidden("dict.item_forbidden", "无权操作此字典项")
    }
}

/// API Key authentication errors (service 层).
pub mod api_key {
    use super::{err_unauthorized, Error};

    /// 401 — API 密钥无效或已失效
    #[must_use]
    pub fn err_invalid() -> Error {
        err_unauthorized("api_key.invalid", "API密钥无效或已失效")
    }

    /// 401 — API 密钥所属租户已停用
    #[must_use]
    pub fn err_tenant_inactive() -> Error {
        err_unauthorized("api_key.tenant_inactive", "API密钥所属租户已停用")
    }

    /// 401 — 超级管理员角色不允许使用 API 密钥
    #[must_use]
    pub fn err_super_admin_not_allowed() -> Error {
        err_unauthorized(
            "api_key.super_admin_not_allowed",
            "超级管理员角色不允许使用API密钥",
        )
    }
}

/// System config errors.
pub mod sys_config {
    use super::{err_forbidden, forbidden, Error, Response, Result};

    /// 403 — 仅超级管理员可管理其他租户的配置
    pub fn super_admin_required() -> Result<Response> {
        forbidden(
            "sys_config.super_admin_required",
            "仅超级管理员可管理其他租户的配置",
        )
    }

    /// 403 — 仅超级管理员可管理其他租户的配置（service/helper 层）
    #[must_use]
    pub fn err_super_admin_required() -> Error {
        err_forbidden(
            "sys_config.super_admin_required",
            "仅超级管理员可管理其他租户的配置",
        )
    }
}

/// i18n errors.
pub mod i18n {
    use super::{forbidden, Response, Result};

    /// 403 — 命名空间不对外公开
    pub fn namespace_not_public(ns: &str) -> Result<Response> {
        forbidden(
            "i18n.namespace_not_public",
            format!("命名空间 '{ns}' 不对外公开"),
        )
    }
}

/// Authentication / token errors (service & extractor layer).
pub mod auth {
    use super::{err_unauthorized, Error};

    /// 401 — 无效的认证令牌
    #[must_use]
    pub fn err_invalid_token() -> Error {
        err_unauthorized("auth.invalid_token", "无效的认证令牌")
    }

    /// 401 — 令牌中的用户ID无效
    #[must_use]
    pub fn err_invalid_user_id() -> Error {
        err_unauthorized("auth.invalid_user_id", "令牌中的用户ID无效")
    }

    /// 401 — 密码已修改，请重新登录
    #[must_use]
    pub fn err_password_changed() -> Error {
        err_unauthorized("auth.password_changed", "密码已修改，请重新登录")
    }

    /// 401 — 令牌中缺少租户编码
    #[must_use]
    pub fn err_missing_tenant_code() -> Error {
        err_unauthorized("auth.missing_tenant_code", "令牌中缺少租户编码")
    }

    /// 401 — 租户不存在
    #[must_use]
    pub fn err_tenant_not_found(code: &str) -> Error {
        err_unauthorized("auth.tenant_not_found", format!("租户不存在: {code}"))
    }

    /// 401 — 租户已停用
    #[must_use]
    pub fn err_tenant_inactive(code: &str) -> Error {
        err_unauthorized("auth.tenant_inactive", format!("租户已停用: {code}"))
    }

    /// 401 — 缺少认证请求头
    #[must_use]
    pub fn err_missing_auth_header() -> Error {
        err_unauthorized("auth.missing_auth_header", "缺少认证请求头")
    }

    /// 401 — 认证请求头格式无效
    #[must_use]
    pub fn err_invalid_auth_header() -> Error {
        err_unauthorized("auth.invalid_auth_header", "认证请求头格式无效")
    }
}
