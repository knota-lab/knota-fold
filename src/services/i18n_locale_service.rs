//! Service for `i18n_supported_locales`.
//!
//! Two read paths:
//! - `list_enabled_cached` — public locale picker, cached 5 min.
//! - `list_all` — admin table (no cache).
//!
//! All writes invalidate the public cache.

use std::time::Duration;

use loco_rs::prelude::*;
use sea_orm::{ActiveValue, EntityTrait};
use uuid::Uuid;

use crate::models::_entities::i18n_supported_locales;
use crate::models::i18n_supported_locales as locales_model;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext};
use crate::views::errors::err_bad_request;
use crate::views::i18n::{
    CreateLocaleRequest, LocaleAdminResponse, LocaleResponse, UpdateLocaleRequest,
};

const CACHE_KEY_ENABLED: &str = "i18n:locales:enabled";
const CACHE_TTL: Duration = Duration::from_secs(300);

/// `BCP-47` validator. Strict but cheap: alnum + `-`, 2-35 chars, must start
/// with a letter, no consecutive `--`, no leading/trailing `-`.
fn validate_locale(locale: &str) -> loco_rs::Result<()> {
    if locale.len() < 2 || locale.len() > 35 {
        return Err(err_bad_request(
            "i18n.locale_length_invalid",
            "locale 长度必须在 2~35 字符之间",
        ));
    }
    if !locale.chars().next().unwrap().is_ascii_alphabetic() {
        return Err(err_bad_request(
            "i18n.locale_must_start_with_letter",
            "locale 必须以字母开头",
        ));
    }
    if locale.starts_with('-') || locale.ends_with('-') || locale.contains("--") {
        return Err(err_bad_request(
            "i18n.locale_dash_invalid",
            "locale 不能以 '-' 开头或结尾，也不能含连续 '--'",
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

// ── Reads ───────────────────────────────────────────────────────────────────

#[tracing::instrument(skip_all)]
pub async fn list_enabled_cached(
    ctx: &AppContext,
) -> loco_rs::Result<Vec<LocaleResponse>> {
    let cached = ctx
        .cache
        .get_or_insert_with_expiry::<Vec<LocaleResponse>, _>(
            CACHE_KEY_ENABLED,
            CACHE_TTL,
            async {
                let rows = locales_model::Model::list_enabled(&ctx.db).await.db_err()?;
                Ok(rows.iter().map(LocaleResponse::from).collect())
            },
        )
        .await?;
    Ok(cached)
}

#[tracing::instrument(skip_all)]
pub async fn list_all(ctx: &AppContext) -> loco_rs::Result<Vec<LocaleAdminResponse>> {
    let rows = locales_model::Model::list_all(&ctx.db).await.db_err()?;
    Ok(rows.iter().map(LocaleAdminResponse::from).collect())
}

// ── Writes ──────────────────────────────────────────────────────────────────

#[tracing::instrument(skip_all)]
pub async fn create(
    ctx: &AppContext,
    _user_id: Uuid,
    params: &CreateLocaleRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<LocaleAdminResponse> {
    validate_locale(&params.locale)?;

    if locales_model::Model::find_by_locale(&ctx.db, &params.locale)
        .await
        .db_err()?
        .is_some()
    {
        return Err(err_bad_request(
            "i18n.locale_exists",
            format!("locale '{}' 已存在", params.locale),
        ));
    }

    let am = i18n_supported_locales::ActiveModel {
        locale: ActiveValue::Set(params.locale.clone()),
        label: ActiveValue::Set(params.label.clone()),
        is_enabled: ActiveValue::Set(params.is_enabled.unwrap_or(true)),
        sort_order: ActiveValue::Set(params.sort_order.unwrap_or(0)),
        ..Default::default()
    };
    let model = am.insert(&ctx.db).await.db_err()?;

    invalidate_cache(ctx).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Create,
        "i18n_locale",
        &model.locale,
        None::<&serde_json::Value>,
        Some(&serde_json::json!({
            "locale": model.locale,
            "label": model.label,
            "isEnabled": model.is_enabled,
        })),
    )
    .await
    .model_err()?;

    Ok(LocaleAdminResponse::from(&model))
}

#[tracing::instrument(skip_all)]
pub async fn update(
    ctx: &AppContext,
    locale: &str,
    _user_id: Uuid,
    params: &UpdateLocaleRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<LocaleAdminResponse> {
    let existing = locales_model::Model::find_by_locale(&ctx.db, locale)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let before = serde_json::json!({
        "locale": existing.locale,
        "label": existing.label,
        "isEnabled": existing.is_enabled,
        "sortOrder": existing.sort_order,
    });

    let mut am = i18n_supported_locales::ActiveModel {
        id: ActiveValue::Unchanged(existing.id),
        ..Default::default()
    };
    if let Some(ref label) = params.label {
        am.label = ActiveValue::Set(label.clone());
    }
    if let Some(is_enabled) = params.is_enabled {
        am.is_enabled = ActiveValue::Set(is_enabled);
    }
    if let Some(sort_order) = params.sort_order {
        am.sort_order = ActiveValue::Set(sort_order);
    }

    let model = am.update(&ctx.db).await.db_err()?;
    invalidate_cache(ctx).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "i18n_locale",
        &model.locale,
        Some(&before),
        Some(&serde_json::json!({
            "locale": model.locale,
            "label": model.label,
            "isEnabled": model.is_enabled,
            "sortOrder": model.sort_order,
        })),
    )
    .await
    .model_err()?;

    Ok(LocaleAdminResponse::from(&model))
}

#[tracing::instrument(skip_all)]
pub async fn delete(
    ctx: &AppContext,
    locale: &str,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    if locale == crate::views::i18n::BASE_LOCALE {
        return Err(err_bad_request(
            "i18n.base_locale_protected",
            format!("禁止删除基础 locale '{locale}'"),
        ));
    }

    let existing = locales_model::Model::find_by_locale(&ctx.db, locale)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let before = serde_json::json!({
        "locale": existing.locale,
        "label": existing.label,
    });

    i18n_supported_locales::Entity::delete_by_id(existing.id)
        .exec(&ctx.db)
        .await
        .db_err()?;

    invalidate_cache(ctx).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "i18n_locale",
        locale,
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(())
}

async fn invalidate_cache(ctx: &AppContext) {
    let _ = ctx.cache.remove(CACHE_KEY_ENABLED).await;
}
