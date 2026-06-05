use std::collections::HashMap;
use std::time::Duration;

use loco_rs::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::models::_entities::sys_configs;
use crate::models::sys_configs as sys_configs_model;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext};
use crate::views::errors::err_bad_request;
use crate::views::sys_configs::{
    ConfigLayers, CreateGlobalConfigRequest, ResolvedConfigDetail, ResolvedConfigSlim,
    ResolvedConfigsResponse, TenantUpsertResponse, UpdateGlobalConfigRequest,
    UpsertTenantConfigRequest,
};

// ── Cache key helpers ──────────────────────────────────────────────────────────

fn cache_key_resolved(tenant_str: &str, key: &str) -> String {
    format!("cfg:resolved:{tenant_str}:{key}")
}

fn cache_key_all(tenant_str: &str) -> String {
    format!("cfg:all:{tenant_str}")
}

fn tenant_str(tenant_id: Option<Uuid>) -> String {
    tenant_id.map_or_else(|| "global".to_string(), |id| id.to_string())
}

// ── Value validation ───────────────────────────────────────────────────────────

/// Validate that `value` is well-formed for the given `value_type`.
fn validate_value(value: &str, value_type: &str) -> loco_rs::Result<()> {
    match value_type {
        "string" => Ok(()),
        "int" => value.parse::<i64>().map(|_| ()).map_err(|_| {
            err_bad_request(
                "sys_config.value_not_int",
                format!("值 '{value}' 不是合法的 int"),
            )
        }),
        "bool" => {
            if value == "true" || value == "false" {
                Ok(())
            } else {
                Err(err_bad_request(
                    "sys_config.value_not_bool",
                    format!(
                        "值 '{value}' 不是合法的 bool（仅接受 \"true\" 或 \"false\"）"
                    ),
                ))
            }
        }
        "json" => serde_json::from_str::<serde_json::Value>(value)
            .map(|_| ())
            .map_err(|_| {
                err_bad_request(
                    "sys_config.value_not_json",
                    format!("值 '{value}' 不是合法的 JSON"),
                )
            }),
        other => Err(err_bad_request(
            "sys_config.value_type_unsupported",
            format!("不支持的 value_type: {other}，合法值为 string | int | bool | json"),
        )),
    }
}

/// Validate key format: [a-z0-9_.], 1-128 chars, no leading/trailing/consecutive dots.
fn validate_key(key: &str) -> loco_rs::Result<()> {
    if key.is_empty() || key.len() > 128 {
        return Err(err_bad_request(
            "sys_config.key_length_invalid",
            "key 长度必须在 1~128 字符之间",
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
    {
        return Err(err_bad_request(
            "sys_config.key_chars_invalid",
            "key 只允许小写字母、数字、下划线和点",
        ));
    }
    if key.starts_with('.') || key.ends_with('.') {
        return Err(err_bad_request(
            "sys_config.key_dot_invalid",
            "key 不能以点开头或结尾",
        ));
    }
    if key.contains("..") {
        return Err(err_bad_request(
            "sys_config.key_consecutive_dots",
            "key 不能包含连续的点",
        ));
    }
    Ok(())
}

/// Validate that category equals the first segment of the key (e.g. "sms" for "sms.enabled").
fn validate_category(key: &str, category: &str) -> loco_rs::Result<()> {
    let expected = key.split('.').next().unwrap_or("");
    if category != expected {
        return Err(err_bad_request(
            "sys_config.category_mismatch",
            format!("category '{category}' 必须等于 key 的第一段 '{expected}'"),
        ));
    }
    Ok(())
}

// ── Read operations ────────────────────────────────────────────────────────────

/// Get fully resolved config for a single key, with source and layers.
/// Used by the debug/admin endpoint `GET /api/sys-configs/resolved/:key`.
#[tracing::instrument(skip_all)]
pub async fn get_resolved_detail(
    ctx: &AppContext,
    key: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<Option<ResolvedConfigDetail>> {
    let ts = tenant_str(tenant_id);
    let cache_key = cache_key_resolved(&ts, key);

    let detail = ctx
        .cache
        .get_or_insert_with_expiry::<ResolvedConfigDetail, _>(
            &cache_key,
            Duration::from_mins(5),
            async {
                resolve_detail_from_db(&ctx.db, key, tenant_id)
                    .await
                    .model_err()
            },
        )
        .await;

    match detail {
        Ok(d) => Ok(Some(d)),
        Err(Error::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Get all resolved configs for the current tenant context.
/// Used by the frontend init endpoint `GET /api/sys-configs/resolved`.
#[tracing::instrument(skip_all)]
pub async fn get_all_resolved(
    ctx: &AppContext,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<ResolvedConfigsResponse> {
    let ts = tenant_str(tenant_id);
    let cache_key = cache_key_all(&ts);

    let configs = ctx
        .cache
        .get_or_insert_with_expiry::<HashMap<String, ResolvedConfigSlim>, _>(
            &cache_key,
            Duration::from_mins(5),
            async {
                fetch_all_resolved_from_db(&ctx.db, tenant_id)
                    .await
                    .model_err()
            },
        )
        .await?;

    Ok(ResolvedConfigsResponse { configs })
}

// ── Write operations ───────────────────────────────────────────────────────────

/// Create a new global config.
#[tracing::instrument(skip_all)]
pub async fn create_global_config(
    ctx: &AppContext,
    user_id: Uuid,
    params: &CreateGlobalConfigRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<sys_configs::Model> {
    validate_key(&params.key)?;
    validate_value(&params.value, &params.value_type)?;
    validate_category(&params.key, &params.category)?;

    // Check for duplicate global key
    if sys_configs_model::Model::find_global_by_key(&ctx.db, &params.key)
        .await
        .db_err()?
        .is_some()
    {
        return Err(err_bad_request(
            "sys_config.key_global_exists",
            format!("全局配置 key '{}' 已存在", params.key),
        ));
    }

    let am = sys_configs::ActiveModel {
        key: ActiveValue::Set(params.key.clone()),
        value: ActiveValue::Set(params.value.clone()),
        value_type: ActiveValue::Set(params.value_type.clone()),
        category: ActiveValue::Set(params.category.clone()),
        scope: ActiveValue::Set("global".to_string()),
        tenant_id: ActiveValue::Set(None),
        label: ActiveValue::Set(params.label.clone()),
        description: ActiveValue::Set(params.description.clone()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };

    let model = am.insert(&ctx.db).await.db_err()?;

    // Invalidate global cache
    let _ = ctx
        .cache
        .remove(&cache_key_resolved("global", &model.key))
        .await;
    let _ = ctx.cache.remove(&cache_key_all("global")).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Create,
        "sys_config",
        &model.key,
        None::<&serde_json::Value>,
        Some(&serde_json::json!({
            "key": model.key,
            "value": model.value,
            "valueType": model.value_type,
        })),
    )
    .await
    .model_err()?;

    Ok(model)
}

/// Update an existing global config.
#[tracing::instrument(skip_all)]
pub async fn update_global_config(
    ctx: &AppContext,
    key: &str,
    user_id: Uuid,
    params: &UpdateGlobalConfigRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<sys_configs::Model> {
    let existing = sys_configs_model::Model::find_global_by_key(&ctx.db, key)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    validate_value(&params.value, &existing.value_type)?;

    let before = serde_json::json!({
        "key": existing.key,
        "value": existing.value,
    });

    let mut am = sys_configs::ActiveModel {
        id: ActiveValue::Unchanged(existing.id),
        value: ActiveValue::Set(params.value.clone()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };
    if let Some(ref label) = params.label {
        am.label = ActiveValue::Set(label.clone());
    }
    if let Some(ref description) = params.description {
        am.description = ActiveValue::Set(Some(description.clone()));
    }

    let model = am.update(&ctx.db).await.db_err()?;

    // Invalidate: single key resolved + global all view
    // v1: tenant resolved caches rely on TTL for global changes
    let _ = ctx.cache.remove(&cache_key_resolved("global", key)).await;
    let _ = ctx.cache.remove(&cache_key_all("global")).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "sys_config",
        key,
        Some(&before),
        Some(&serde_json::json!({
            "key": model.key,
            "value": model.value,
        })),
    )
    .await
    .model_err()?;

    Ok(model)
}

/// Delete a global config and cascade-delete all tenant overrides for that key.
#[tracing::instrument(skip_all)]
pub async fn delete_global_config(
    ctx: &AppContext,
    key: &str,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = sys_configs_model::Model::find_global_by_key(&ctx.db, key)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let before = serde_json::json!({
        "key": existing.key,
        "value": existing.value,
    });

    // Cascade: delete all tenant overrides first (logical FK)
    sys_configs_model::Model::delete_tenant_overrides_for_key(&ctx.db, key)
        .await
        .db_err()?;

    sys_configs::Entity::delete_by_id(existing.id)
        .exec(&ctx.db)
        .await
        .db_err()?;

    // Invalidate global cache
    let _ = ctx.cache.remove(&cache_key_resolved("global", key)).await;
    let _ = ctx.cache.remove(&cache_key_all("global")).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "sys_config",
        key,
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(())
}

/// Upsert a tenant override for a key. The global config for this key must exist.
#[tracing::instrument(skip_all)]
pub async fn upsert_tenant_config(
    ctx: &AppContext,
    key: &str,
    tenant_id: Uuid,
    user_id: Uuid,
    params: &UpsertTenantConfigRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<TenantUpsertResponse> {
    // Global config must exist — value_type is inherited from it
    let global = sys_configs_model::Model::find_global_by_key(&ctx.db, key)
        .await
        .db_err()?
        .ok_or_else(|| {
            err_bad_request(
                "sys_config.key_global_not_found",
                format!("全局配置 key '{key}' 不存在"),
            )
        })?;

    validate_value(&params.value, &global.value_type)?;

    let ts = tenant_str(Some(tenant_id));

    let existing_override =
        sys_configs_model::Model::find_tenant_by_key(&ctx.db, key, tenant_id)
            .await
            .db_err()?;

    let before = existing_override
        .as_ref()
        .map(|m| serde_json::json!({ "key": m.key, "value": m.value }));

    if let Some(existing) = existing_override {
        let am = sys_configs::ActiveModel {
            id: ActiveValue::Unchanged(existing.id),
            value: ActiveValue::Set(params.value.clone()),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.update(&ctx.db).await.db_err()?;
    } else {
        let am = sys_configs::ActiveModel {
            key: ActiveValue::Set(key.to_string()),
            value: ActiveValue::Set(params.value.clone()),
            value_type: ActiveValue::Set(global.value_type.clone()),
            category: ActiveValue::Set(global.category.clone()),
            scope: ActiveValue::Set("tenant".to_string()),
            tenant_id: ActiveValue::Set(Some(tenant_id)),
            label: ActiveValue::Set(global.label.clone()),
            description: ActiveValue::Set(global.description.clone()),
            updated_by: ActiveValue::Set(Some(user_id)),
            ..Default::default()
        };
        am.insert(&ctx.db).await.db_err()?;
    }

    // Invalidate tenant resolved caches
    let _ = ctx.cache.remove(&cache_key_resolved(&ts, key)).await;
    let _ = ctx.cache.remove(&cache_key_all(&ts)).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Update,
        "sys_config_tenant",
        key,
        before.as_ref(),
        Some(&serde_json::json!({
            "key": key,
            "value": params.value,
            "tenantId": tenant_id.to_string(),
        })),
    )
    .await
    .model_err()?;

    // Return resolved view
    let response = TenantUpsertResponse {
        key: key.to_string(),
        resolved_value: params.value.clone(),
        value_type: global.value_type.clone(),
        source: "TENANT_DB".to_string(),
        layers: ConfigLayers {
            tenant: Some(params.value.clone()),
            global: Some(global.value.clone()),
        },
    };

    Ok(response)
}

/// Delete a tenant override for a key, reverting to the global default.
#[tracing::instrument(skip_all)]
pub async fn delete_tenant_config(
    ctx: &AppContext,
    key: &str,
    tenant_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = sys_configs_model::Model::find_tenant_by_key(&ctx.db, key, tenant_id)
        .await
        .db_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let before = serde_json::json!({
        "key": existing.key,
        "value": existing.value,
        "tenantId": tenant_id.to_string(),
    });

    sys_configs::Entity::delete_by_id(existing.id)
        .exec(&ctx.db)
        .await
        .db_err()?;

    let ts = tenant_str(Some(tenant_id));
    let _ = ctx.cache.remove(&cache_key_resolved(&ts, key)).await;
    let _ = ctx.cache.remove(&cache_key_all(&ts)).await;

    audit_service::log(
        &ctx.db,
        audit_ctx,
        AuditAction::Delete,
        "sys_config_tenant",
        key,
        Some(&before),
        None::<&serde_json::Value>,
    )
    .await
    .model_err()?;

    Ok(())
}

/// List global configs with optional category/prefix filtering and pagination.
#[tracing::instrument(skip_all)]
pub async fn list_global_configs(
    db: &DatabaseConnection,
    category: Option<&str>,
    prefix: Option<&str>,
    page: u64,
    page_size: u64,
) -> loco_rs::Result<(Vec<sys_configs::Model>, u64)> {
    use sea_orm::{PaginatorTrait, QueryOrder, QuerySelect};

    let mut query =
        sys_configs::Entity::find().filter(sys_configs::Column::TenantId.is_null());

    if let Some(cat) = category {
        query = query.filter(sys_configs::Column::Category.eq(cat));
    }
    if let Some(pfx) = prefix {
        query = query.filter(sys_configs::Column::Key.starts_with(pfx));
    }

    let total = query.clone().count(db).await.db_err()?;

    let items = query
        .order_by_asc(sys_configs::Column::Category)
        .order_by_asc(sys_configs::Column::Key)
        .offset((page - 1) * page_size)
        .limit(page_size)
        .all(db)
        .await
        .db_err()?;

    Ok((items, total))
}

/// List tenant override configs with optional category/prefix filtering.
#[tracing::instrument(skip_all)]
pub async fn list_tenant_configs(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    category: Option<&str>,
    prefix: Option<&str>,
) -> loco_rs::Result<Vec<sys_configs::Model>> {
    use sea_orm::QueryOrder;

    let mut query =
        sys_configs::Entity::find().filter(sys_configs::Column::TenantId.eq(tenant_id));

    if let Some(cat) = category {
        query = query.filter(sys_configs::Column::Category.eq(cat));
    }
    if let Some(pfx) = prefix {
        query = query.filter(sys_configs::Column::Key.starts_with(pfx));
    }

    let items = query
        .order_by_asc(sys_configs::Column::Category)
        .order_by_asc(sys_configs::Column::Key)
        .all(db)
        .await
        .db_err()?;

    Ok(items)
}

// ── Private DB helpers ─────────────────────────────────────────────────────────

async fn resolve_detail_from_db(
    db: &DatabaseConnection,
    key: &str,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<ResolvedConfigDetail> {
    let global = sys_configs_model::Model::find_global_by_key(db, key)
        .await
        .db_err()?;

    let tenant_val = if let Some(tid) = tenant_id {
        sys_configs_model::Model::find_tenant_by_key(db, key, tid)
            .await
            .db_err()?
            .map(|m| m.value)
    } else {
        None
    };

    match (&global, &tenant_val) {
        (None, None) => Err(crate::views::errors::err_not_found(
            "sys_config.not_found",
            "配置项不存在",
        )),
        (Some(g), None) => Ok(ResolvedConfigDetail {
            key: key.to_string(),
            resolved_value: g.value.clone(),
            value_type: g.value_type.clone(),
            source: "GLOBAL_DB".to_string(),
            layers: ConfigLayers {
                tenant: None,
                global: Some(g.value.clone()),
            },
        }),
        (Some(g), Some(tv)) => Ok(ResolvedConfigDetail {
            key: key.to_string(),
            resolved_value: tv.clone(),
            value_type: g.value_type.clone(),
            source: "TENANT_DB".to_string(),
            layers: ConfigLayers {
                tenant: Some(tv.clone()),
                global: Some(g.value.clone()),
            },
        }),
        (None, Some(_)) => {
            // Orphan tenant override (global was deleted) — should not happen, but handle gracefully
            Err(crate::views::errors::err_not_found(
                "sys_config.orphan_override",
                "租户配置覆盖项孤立（全局配置已删除）",
            ))
        }
    }
}

async fn fetch_all_resolved_from_db(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
) -> loco_rs::Result<HashMap<String, ResolvedConfigSlim>> {
    let globals = sys_configs_model::Model::list_global(db).await.db_err()?;

    let mut map: HashMap<String, ResolvedConfigSlim> = globals
        .iter()
        .map(|g| {
            (
                g.key.clone(),
                ResolvedConfigSlim {
                    value: g.value.clone(),
                    value_type: g.value_type.clone(),
                    source: "GLOBAL_DB".to_string(),
                },
            )
        })
        .collect();

    if let Some(tid) = tenant_id {
        let overrides = sys_configs_model::Model::list_tenant_overrides(db, tid)
            .await
            .db_err()?;

        for o in &overrides {
            // Only override keys that have a corresponding global config
            if map.contains_key(&o.key) {
                map.insert(
                    o.key.clone(),
                    ResolvedConfigSlim {
                        value: o.value.clone(),
                        value_type: o.value_type.clone(),
                        source: "TENANT_DB".to_string(),
                    },
                );
            }
        }
    }

    Ok(map)
}
