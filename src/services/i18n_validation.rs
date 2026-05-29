//! Validation helpers and tenant namespace policy for i18n operations.

use axum::http::StatusCode;
use std::collections::HashSet;

use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::*;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::models::i18n_queries;
use crate::models::tenants;
use crate::utils::error::IntoAppError;
use crate::views::errors::err_bad_request;

/// Namespace: PascalCase or `Tenant.<sub>`, 1-64 chars, alnum + `.`.
pub(crate) fn validate_namespace(ns: &str) -> loco_rs::Result<()> {
    if ns.is_empty() || ns.len() > 64 {
        return Err(err_bad_request(
            "i18n.namespace_length_invalid",
            "namespace 长度必须在 1~64 字符之间",
        ));
    }
    if !ns
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return Err(err_bad_request(
            "i18n.namespace_chars_invalid",
            "namespace 仅允许字母、数字、'.'、'_'",
        ));
    }
    Ok(())
}

pub(crate) fn tenant_namespace_prefix(tenant_code: &str) -> String {
    format!("Tenant.{tenant_code}")
}

pub(crate) fn namespace_has_prefix(namespace: &str, prefix: &str) -> bool {
    namespace == prefix || namespace.starts_with(&format!("{prefix}."))
}

pub(crate) async fn load_tenant_namespace_policy(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> loco_rs::Result<(String, HashSet<String>)> {
    let tenant = tenants::Model::find_by_id(db, tenant_id)
        .await
        .map_err(|_| {
            Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("tenant.not_found", "租户未找到"),
            )
        })?;
    let readable_namespaces = i18n_queries::list_tenant_namespaces(db, tenant_id)
        .await
        .db_err()?
        .into_iter()
        .map(|row| row.namespace)
        .collect();

    Ok((tenant_namespace_prefix(&tenant.code), readable_namespaces))
}

/// Tenant namespace policy used by tenant override writes and import/export.
///
/// A tenant may operate on namespaces that either:
/// - start with that tenant's own private prefix (`Tenant.{tenant_code}`); or
/// - are already readable to that tenant (the union of global namespaces plus
///   the tenant's existing override namespaces).
///
/// This keeps import/export behavior aligned with the bundle/listing model
/// while still blocking arbitrary non-readable system namespaces.
pub(crate) fn ensure_tenant_namespace_allowed(
    namespace: &str,
    own_prefix: &str,
    readable_namespaces: &HashSet<String>,
) -> loco_rs::Result<()> {
    if namespace_has_prefix(namespace, own_prefix)
        || readable_namespaces.contains(namespace)
    {
        return Ok(());
    }

    Err(err_bad_request("i18n.tenant_namespace_forbidden", format!(
        "namespace '{namespace}' 对当前租户不可访问；仅允许 {own_prefix}.* 或租户可读 namespace"
    )))
}

/// Key: alnum + `._-`, 1-256 chars.
pub(crate) fn validate_key(key: &str) -> loco_rs::Result<()> {
    if key.is_empty() || key.len() > 256 {
        return Err(err_bad_request(
            "i18n.key_length_invalid",
            "key 长度必须在 1~256 字符之间",
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(err_bad_request(
            "i18n.key_chars_invalid",
            "key 仅允许字母、数字、'.'、'_'、'-'",
        ));
    }
    Ok(())
}

/// Locale string: 2-35 chars, BCP-47 lite.
pub(crate) fn validate_locale(locale: &str) -> loco_rs::Result<()> {
    if locale.len() < 2 || locale.len() > 35 {
        return Err(err_bad_request(
            "i18n.locale_length_invalid",
            "locale 长度必须在 2~35 字符之间",
        ));
    }
    if !locale
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(err_bad_request(
            "i18n.locale_chars_invalid",
            "locale 仅允许字母、数字与 '-'",
        ));
    }
    Ok(())
}
