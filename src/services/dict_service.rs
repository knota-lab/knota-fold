use loco_rs::prelude::model::query;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

use crate::models::_entities::{dict_items, dict_types};
use crate::models::dict_items as dict_items_model;
use crate::models::dict_items::EffectiveDictItem;
use crate::models::dict_types as dict_types_model;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult};
use crate::views::audit_logs::{
    AuditAction, AuditContext, DictItemAuditSnapshot, DictTypeAuditSnapshot,
};
use crate::views::dicts::{
    CreateDictItemRequest, CreateDictTypeRequest, DictItemResponse, DictItemTreeResponse,
    DictTypeResponse, UpdateDictItemRequest, UpdateDictTypeRequest,
};
use crate::views::errors::err_bad_request;
use crate::views::pagination::PaginatedResponse;

// ══════════════════════════════════════════════
//  Dict Type Operations
// ══════════════════════════════════════════════

#[tracing::instrument(skip_all)]
pub async fn list_dict_types(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    pagination: &query::PaginationQuery,
) -> loco_rs::Result<PaginatedResponse<DictTypeResponse>> {
    let page = pagination.page;
    let page_size = pagination.page_size;

    match tenant_id {
        // Super admin: system types only
        None => {
            let (rows, total) =
                dict_types_model::Model::find_system_types_paginated(db, page, page_size)
                    .await
                    .model_err()?;
            let total_pages = if total == 0 {
                0
            } else {
                total.div_ceil(page_size)
            };
            Ok(PaginatedResponse {
                items: rows.iter().map(DictTypeResponse::from_model).collect(),
                total_pages,
                total_items: total,
                page,
                page_size,
            })
        }
        // Tenant: effective merge view
        Some(tid) => {
            let (rows, total) =
                dict_types_model::Model::find_effective_types(db, tid, page, page_size)
                    .await
                    .model_err()?;
            let total_pages = if total == 0 {
                0
            } else {
                total.div_ceil(page_size)
            };
            Ok(PaginatedResponse {
                items: rows.iter().map(DictTypeResponse::from_effective).collect(),
                total_pages,
                total_items: total,
                page,
                page_size,
            })
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn create_dict_type(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    user_id: Uuid,
    params: &CreateDictTypeRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<dict_types::Model> {
    // Tenant cannot create code conflicting with system dict
    if tenant_id.is_some()
        && dict_types_model::Model::find_system_type_by_code(db, &params.code)
            .await
            .is_ok()
    {
        return Err(err_bad_request(
            "dict.type_code_conflict",
            "字典类型编码与系统字典冲突",
        ));
    }

    let mut am = dict_types_model::ActiveModel {
        code: ActiveValue::Set(params.code.clone()),
        name: ActiveValue::Set(params.name.clone()),
        ..Default::default()
    };
    if let Some(ref description) = params.description {
        am.description = ActiveValue::Set(Some(description.clone()));
    }

    let dict_type = dict_types_model::Model::create_dict_type(db, tenant_id, am, user_id)
        .await
        .model_err()?;

    let snapshot = DictTypeAuditSnapshot::from(&dict_type);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "dict_type",
        &dict_type.id.to_string(),
        None::<&DictTypeAuditSnapshot>,
        Some(&snapshot),
    )
    .await
    .model_err()?;

    Ok(dict_type)
}

#[tracing::instrument(skip_all)]
pub async fn update_dict_type(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Option<Uuid>,
    user_id: Uuid,
    params: &UpdateDictTypeRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<dict_types::Model> {
    let existing = dict_types_model::Model::find_by_id(db, id)
        .await
        .model_err()?;

    match tenant_id {
        // Super admin: direct update on system row only
        None => {
            if existing.tenant_id.is_some() {
                return Err(err_bad_request(
                    "dict.type_system_only",
                    "超级管理员只能编辑系统字典类型",
                ));
            }
            let before = DictTypeAuditSnapshot::from(&existing);
            let updated = apply_dict_type_update(db, id, user_id, params).await?;
            let after = DictTypeAuditSnapshot::from(&updated);
            audit_service::log(
                db,
                audit_ctx,
                AuditAction::Update,
                "dict_type",
                &updated.id.to_string(),
                Some(&before),
                Some(&after),
            )
            .await
            .model_err()?;
            Ok(updated)
        }
        Some(tid) => {
            if existing.tenant_id.is_none() {
                let before = dict_types_model::Model::find_override_by_tenant_and_source(
                    db,
                    tid,
                    existing.id,
                )
                .await
                .ok()
                .map(|override_row| DictTypeAuditSnapshot::from(&override_row));
                let updated =
                    create_type_override(db, &existing, tid, user_id, Some(params))
                        .await?;
                let after = DictTypeAuditSnapshot::from(&updated);
                audit_service::log(
                    db,
                    audit_ctx,
                    AuditAction::Update,
                    "dict_type",
                    &updated.id.to_string(),
                    before.as_ref(),
                    Some(&after),
                )
                .await
                .model_err()?;
                Ok(updated)
            } else if existing.tenant_id == Some(tid) {
                // Own row (override or tenant_only) → direct update
                let before = DictTypeAuditSnapshot::from(&existing);
                let updated = apply_dict_type_update(db, id, user_id, params).await?;
                let after = DictTypeAuditSnapshot::from(&updated);
                audit_service::log(
                    db,
                    audit_ctx,
                    AuditAction::Update,
                    "dict_type",
                    &updated.id.to_string(),
                    Some(&before),
                    Some(&after),
                )
                .await
                .model_err()?;
                Ok(updated)
            } else {
                Err(crate::views::errors::dict::err_type_forbidden())
            }
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn toggle_dict_type_status(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Option<Uuid>,
    user_id: Uuid,
    version: i32,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<dict_types::Model> {
    let existing = dict_types_model::Model::find_by_id(db, id)
        .await
        .model_err()?;

    match tenant_id {
        // Super admin: toggle system row directly
        None => {
            if existing.tenant_id.is_some() {
                return Err(err_bad_request(
                    "dict.type_system_only_toggle",
                    "超级管理员只能操作系统字典类型",
                ));
            }
            let new_status = flip_status(&existing.status);
            let am = dict_types_model::ActiveModel {
                status: ActiveValue::Set(new_status),
                version: ActiveValue::Set(version),
                ..Default::default()
            };
            let before = DictTypeAuditSnapshot::from(&existing);
            let updated =
                dict_types_model::Model::update_with_version(db, id, am, user_id)
                    .await
                    .model_err()?;
            let after = DictTypeAuditSnapshot::from(&updated);
            audit_service::log(
                db,
                audit_ctx,
                AuditAction::Update,
                "dict_type",
                &updated.id.to_string(),
                Some(&before),
                Some(&after),
            )
            .await
            .model_err()?;
            Ok(updated)
        }
        Some(tid) => {
            if existing.tenant_id.is_none() {
                // System row → check for existing override first
                if let Ok(override_row) =
                    dict_types_model::Model::find_override_by_tenant_and_source(
                        db,
                        tid,
                        existing.id,
                    )
                    .await
                {
                    // Toggle existing override
                    let before = DictTypeAuditSnapshot::from(&override_row);
                    let new_status = flip_status(&override_row.status);
                    let am = dict_types_model::ActiveModel {
                        status: ActiveValue::Set(new_status),
                        version: ActiveValue::Set(version),
                        ..Default::default()
                    };
                    let updated = dict_types_model::Model::update_with_version(
                        db,
                        override_row.id,
                        am,
                        user_id,
                    )
                    .await
                    .model_err()?;
                    let after = DictTypeAuditSnapshot::from(&updated);
                    audit_service::log(
                        db,
                        audit_ctx,
                        AuditAction::Update,
                        "dict_type",
                        &updated.id.to_string(),
                        Some(&before),
                        Some(&after),
                    )
                    .await
                    .model_err()?;
                    Ok(updated)
                } else {
                    // No override yet → create one with disabled status
                    let updated = create_type_override_with_status(
                        db, &existing, tid, user_id, "disabled",
                    )
                    .await?;
                    let after = DictTypeAuditSnapshot::from(&updated);
                    audit_service::log(
                        db,
                        audit_ctx,
                        AuditAction::Update,
                        "dict_type",
                        &updated.id.to_string(),
                        None::<&DictTypeAuditSnapshot>,
                        Some(&after),
                    )
                    .await
                    .model_err()?;
                    Ok(updated)
                }
            } else if existing.tenant_id == Some(tid) {
                // Own row → direct toggle
                let before = DictTypeAuditSnapshot::from(&existing);
                let new_status = flip_status(&existing.status);
                let am = dict_types_model::ActiveModel {
                    status: ActiveValue::Set(new_status),
                    version: ActiveValue::Set(version),
                    ..Default::default()
                };
                let updated =
                    dict_types_model::Model::update_with_version(db, id, am, user_id)
                        .await
                        .model_err()?;
                let after = DictTypeAuditSnapshot::from(&updated);
                audit_service::log(
                    db,
                    audit_ctx,
                    AuditAction::Update,
                    "dict_type",
                    &updated.id.to_string(),
                    Some(&before),
                    Some(&after),
                )
                .await
                .model_err()?;
                Ok(updated)
            } else {
                Err(crate::views::errors::dict::err_type_forbidden())
            }
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn reset_dict_type_override(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = dict_types_model::Model::find_by_id(db, id)
        .await
        .model_err()?;

    // Must be an override row owned by this tenant
    if existing.tenant_id != Some(tenant_id) || existing.source_type_id.is_none() {
        return Err(err_bad_request(
            "dict.type_override_only",
            "只能重置租户的覆盖字典类型",
        ));
    }

    let before = DictTypeAuditSnapshot::from(&existing);

    // Soft-delete the type override
    dict_types_model::Model::soft_delete(db, id)
        .await
        .model_err()?;

    // Cascade: soft-delete all override items this tenant has for the source type
    let source_type_id = existing.source_type_id.unwrap();
    let override_items = dict_items_model::Model::find_tenant_overrides_for_type(
        db,
        tenant_id,
        source_type_id,
    )
    .await
    .model_err()?;

    for item in override_items {
        dict_items_model::Model::soft_delete(db, item.id)
            .await
            .model_err()?;
    }

    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Delete,
        "dict_type",
        &existing.id.to_string(),
        Some(&before),
        None::<&DictTypeAuditSnapshot>,
    )
    .await
    .model_err()?;

    Ok(())
}

// ══════════════════════════════════════════════
//  Dict Item Operations
// ══════════════════════════════════════════════

#[tracing::instrument(skip_all)]
pub async fn list_dict_items(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    type_code: &str,
) -> loco_rs::Result<Vec<DictItemResponse>> {
    match tenant_id {
        None => {
            // Super admin: system items only
            let system_type =
                dict_types_model::Model::find_system_type_by_code(db, type_code)
                    .await
                    .model_err()?;
            let items =
                dict_items_model::Model::find_system_items_by_type(db, system_type.id)
                    .await
                    .model_err()?;
            Ok(items.iter().map(DictItemResponse::from_model).collect())
        }
        Some(tid) => {
            // Tenant: effective merge
            let base_type_id = resolve_type_id_by_code(db, Some(tid), type_code).await?;
            let items =
                dict_items_model::Model::find_effective_items(db, tid, base_type_id)
                    .await
                    .model_err()?;
            Ok(items.iter().map(DictItemResponse::from_effective).collect())
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn get_dict_item_tree(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    type_code: &str,
) -> loco_rs::Result<Vec<DictItemTreeResponse>> {
    match tenant_id {
        None => {
            // Super admin: system items tree
            let system_type =
                dict_types_model::Model::find_system_type_by_code(db, type_code)
                    .await
                    .model_err()?;
            let items =
                dict_items_model::Model::find_system_items_by_type(db, system_type.id)
                    .await
                    .model_err()?;
            Ok(build_model_tree(&items, None))
        }
        Some(tid) => {
            // Tenant: effective items tree (uses base_item_id for parent matching)
            let base_type_id = resolve_type_id_by_code(db, Some(tid), type_code).await?;
            let items =
                dict_items_model::Model::find_effective_items(db, tid, base_type_id)
                    .await
                    .model_err()?;
            Ok(build_effective_tree(&items, None))
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn create_dict_item(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    user_id: Uuid,
    params: &CreateDictItemRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<dict_items::Model> {
    // Resolve base type id (if override type, follow source_type_id)
    let base_type_id = resolve_base_type_id(db, params.dict_type_id)
        .await
        .model_err()?;

    // Validate tree depth if parent specified
    if let Some(parent_id) = params.parent_id {
        dict_items_model::Model::validate_tree_depth(db, Some(parent_id))
            .await
            .model_err()?;
    }

    let mut am = dict_items_model::ActiveModel {
        dict_type_id: ActiveValue::Set(base_type_id),
        code: ActiveValue::Set(params.code.clone()),
        name: ActiveValue::Set(params.name.clone()),
        value: ActiveValue::Set(params.value.clone()),
        ..Default::default()
    };
    if let Some(parent_id) = params.parent_id {
        am.parent_id = ActiveValue::Set(Some(parent_id));
    }
    if let Some(sort_order) = params.sort_order {
        am.sort_order = ActiveValue::Set(sort_order);
    }
    if let Some(ref description) = params.description {
        am.description = ActiveValue::Set(Some(description.clone()));
    }

    let item = dict_items_model::Model::create_dict_item(db, tenant_id, am, user_id)
        .await
        .model_err()?;

    let snapshot = DictItemAuditSnapshot::from(&item);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "dict_item",
        &item.id.to_string(),
        None::<&DictItemAuditSnapshot>,
        Some(&snapshot),
    )
    .await
    .model_err()?;

    Ok(item)
}

#[tracing::instrument(skip_all)]
pub async fn update_dict_item(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Option<Uuid>,
    user_id: Uuid,
    params: &UpdateDictItemRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<dict_items::Model> {
    let existing = dict_items_model::Model::find_by_id(db, id)
        .await
        .model_err()?;

    // Validate tree constraints if parent changes
    if let Some(Some(pid)) = params.parent_id {
        dict_items_model::Model::validate_no_circular_ref(db, id, Some(pid))
            .await
            .model_err()?;
        dict_items_model::Model::validate_tree_depth(db, Some(pid))
            .await
            .model_err()?;
    }

    match tenant_id {
        // Super admin: direct update on system row only
        None => {
            if existing.tenant_id.is_some() {
                return Err(err_bad_request(
                    "dict.item_system_only",
                    "超级管理员只能编辑系统字典项",
                ));
            }
            let before = DictItemAuditSnapshot::from(&existing);
            let updated = apply_dict_item_update(db, id, user_id, params).await?;
            let after = DictItemAuditSnapshot::from(&updated);
            audit_service::log(
                db,
                audit_ctx,
                AuditAction::Update,
                "dict_item",
                &updated.id.to_string(),
                Some(&before),
                Some(&after),
            )
            .await
            .model_err()?;
            Ok(updated)
        }
        Some(tid) => {
            if existing.tenant_id.is_none() {
                // System row → copy-on-write: create override item
                let before = dict_items_model::Model::find_override_by_tenant_and_source(
                    db,
                    tid,
                    existing.id,
                )
                .await
                .ok()
                .map(|override_row| DictItemAuditSnapshot::from(&override_row));
                let updated =
                    create_item_override(db, &existing, tid, user_id, Some(params))
                        .await?;
                let after = DictItemAuditSnapshot::from(&updated);
                audit_service::log(
                    db,
                    audit_ctx,
                    AuditAction::Update,
                    "dict_item",
                    &updated.id.to_string(),
                    before.as_ref(),
                    Some(&after),
                )
                .await
                .model_err()?;
                Ok(updated)
            } else if existing.tenant_id == Some(tid) {
                // Own row → direct update
                let before = DictItemAuditSnapshot::from(&existing);
                let updated = apply_dict_item_update(db, id, user_id, params).await?;
                let after = DictItemAuditSnapshot::from(&updated);
                audit_service::log(
                    db,
                    audit_ctx,
                    AuditAction::Update,
                    "dict_item",
                    &updated.id.to_string(),
                    Some(&before),
                    Some(&after),
                )
                .await
                .model_err()?;
                Ok(updated)
            } else {
                Err(crate::views::errors::dict::err_item_forbidden())
            }
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn toggle_dict_item_status(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Option<Uuid>,
    user_id: Uuid,
    version: i32,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<dict_items::Model> {
    let existing = dict_items_model::Model::find_by_id(db, id)
        .await
        .model_err()?;

    match tenant_id {
        // Super admin: toggle system row directly
        None => {
            if existing.tenant_id.is_some() {
                return Err(err_bad_request(
                    "dict.item_system_only_toggle",
                    "超级管理员只能操作系统字典项",
                ));
            }
            let new_status = flip_status(&existing.status);
            let am = dict_items_model::ActiveModel {
                status: ActiveValue::Set(new_status),
                version: ActiveValue::Set(version),
                ..Default::default()
            };
            let before = DictItemAuditSnapshot::from(&existing);
            let updated =
                dict_items_model::Model::update_with_version(db, id, am, user_id)
                    .await
                    .model_err()?;
            let after = DictItemAuditSnapshot::from(&updated);
            audit_service::log(
                db,
                audit_ctx,
                AuditAction::Update,
                "dict_item",
                &updated.id.to_string(),
                Some(&before),
                Some(&after),
            )
            .await
            .model_err()?;
            Ok(updated)
        }
        Some(tid) => {
            if existing.tenant_id.is_none() {
                // System row → check for existing override
                if let Ok(override_row) =
                    dict_items_model::Model::find_override_by_tenant_and_source(
                        db,
                        tid,
                        existing.id,
                    )
                    .await
                {
                    // Toggle existing override
                    let before = DictItemAuditSnapshot::from(&override_row);
                    let new_status = flip_status(&override_row.status);
                    let am = dict_items_model::ActiveModel {
                        status: ActiveValue::Set(new_status),
                        version: ActiveValue::Set(version),
                        ..Default::default()
                    };
                    let updated = dict_items_model::Model::update_with_version(
                        db,
                        override_row.id,
                        am,
                        user_id,
                    )
                    .await
                    .model_err()?;
                    let after = DictItemAuditSnapshot::from(&updated);
                    audit_service::log(
                        db,
                        audit_ctx,
                        AuditAction::Update,
                        "dict_item",
                        &updated.id.to_string(),
                        Some(&before),
                        Some(&after),
                    )
                    .await
                    .model_err()?;
                    Ok(updated)
                } else {
                    // No override yet → create one with disabled status
                    let updated = create_item_override_with_status(
                        db, &existing, tid, user_id, "disabled",
                    )
                    .await?;
                    let after = DictItemAuditSnapshot::from(&updated);
                    audit_service::log(
                        db,
                        audit_ctx,
                        AuditAction::Update,
                        "dict_item",
                        &updated.id.to_string(),
                        None::<&DictItemAuditSnapshot>,
                        Some(&after),
                    )
                    .await
                    .model_err()?;
                    Ok(updated)
                }
            } else if existing.tenant_id == Some(tid) {
                // Own row → direct toggle
                let before = DictItemAuditSnapshot::from(&existing);
                let new_status = flip_status(&existing.status);
                let am = dict_items_model::ActiveModel {
                    status: ActiveValue::Set(new_status),
                    version: ActiveValue::Set(version),
                    ..Default::default()
                };
                let updated =
                    dict_items_model::Model::update_with_version(db, id, am, user_id)
                        .await
                        .model_err()?;
                let after = DictItemAuditSnapshot::from(&updated);
                audit_service::log(
                    db,
                    audit_ctx,
                    AuditAction::Update,
                    "dict_item",
                    &updated.id.to_string(),
                    Some(&before),
                    Some(&after),
                )
                .await
                .model_err()?;
                Ok(updated)
            } else {
                Err(crate::views::errors::dict::err_item_forbidden())
            }
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn reset_dict_item_override(
    db: &DatabaseConnection,
    id: Uuid,
    tenant_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = dict_items_model::Model::find_by_id(db, id)
        .await
        .model_err()?;

    // Must be an override row owned by this tenant
    if existing.tenant_id != Some(tenant_id) || existing.source_item_id.is_none() {
        return Err(err_bad_request(
            "dict.item_override_only",
            "只能重置租户的覆盖字典项",
        ));
    }

    let before = DictItemAuditSnapshot::from(&existing);

    dict_items_model::Model::soft_delete(db, id)
        .await
        .model_err()?;

    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Delete,
        "dict_item",
        &existing.id.to_string(),
        Some(&before),
        None::<&DictItemAuditSnapshot>,
    )
    .await
    .model_err()?;

    Ok(())
}

// ══════════════════════════════════════════════
//  Private helpers
// ══════════════════════════════════════════════

fn flip_status(current: &str) -> String {
    if current == "active" {
        "disabled".to_string()
    } else {
        "active".to_string()
    }
}

/// Resolve a `type_code` to the base `dict_type_id` for item queries.
/// System types take precedence; falls back to `tenant_only` type.
async fn resolve_type_id_by_code(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    type_code: &str,
) -> loco_rs::Result<Uuid> {
    // Try system type first
    if let Ok(system_type) =
        dict_types_model::Model::find_system_type_by_code(db, type_code).await
    {
        return Ok(system_type.id);
    }

    // For tenant: try tenant_only type
    if let Some(tid) = tenant_id {
        let tenant_type = dict_types::Entity::find()
            .filter(dict_types::Column::TenantId.eq(tid))
            .filter(dict_types::Column::Code.eq(type_code))
            .filter(dict_types::Column::SourceTypeId.is_null())
            .filter(dict_types::Column::DeletedAt.is_null())
            .one(db)
            .await
            .db_err()?;

        if let Some(t) = tenant_type {
            return Ok(t.id);
        }
    }

    Err(crate::views::errors::err_not_found(
        "dict.tenant_type_not_found",
        "租户类型未找到",
    ))
}

/// Resolve a `type_id` to base `type_id` (follow `source_type_id` if override type).
async fn resolve_base_type_id(
    db: &DatabaseConnection,
    type_id: Uuid,
) -> loco_rs::Result<Uuid> {
    let dict_type = dict_types_model::Model::find_by_id(db, type_id)
        .await
        .model_err()?;
    Ok(dict_type.source_type_id.unwrap_or(dict_type.id))
}

/// Apply mutable field updates to a dict type (used for both super admin and tenant own rows).
async fn apply_dict_type_update(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
    params: &UpdateDictTypeRequest,
) -> loco_rs::Result<dict_types::Model> {
    let mut am = dict_types_model::ActiveModel {
        version: ActiveValue::Set(params.version),
        ..Default::default()
    };
    if let Some(ref name) = params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(ref description) = params.description {
        am.description = ActiveValue::Set(description.clone());
    }

    dict_types_model::Model::update_with_version(db, id, am, user_id)
        .await
        .model_err()
}

/// Create a dict type override from a system row (copy-on-write).
/// If an override already exists for this tenant+source, update it instead.
async fn create_type_override(
    db: &DatabaseConnection,
    system_row: &dict_types::Model,
    tenant_id: Uuid,
    user_id: Uuid,
    params: Option<&UpdateDictTypeRequest>,
) -> loco_rs::Result<dict_types::Model> {
    // Check if override already exists
    if let Ok(existing_override) =
        dict_types_model::Model::find_override_by_tenant_and_source(
            db,
            tenant_id,
            system_row.id,
        )
        .await
    {
        // Already has override — update it instead
        if let Some(p) = params {
            return apply_dict_type_update(db, existing_override.id, user_id, p).await;
        }
        return Ok(existing_override);
    }

    let name = params
        .and_then(|p| p.name.as_ref())
        .unwrap_or(&system_row.name);
    let description = params
        .and_then(|p| p.description.as_ref())
        .unwrap_or(&system_row.description);

    let am = dict_types_model::ActiveModel {
        code: ActiveValue::Set(system_row.code.clone()),
        name: ActiveValue::Set(name.clone()),
        source_type_id: ActiveValue::Set(Some(system_row.id)),
        description: ActiveValue::Set(description.clone()),
        ..Default::default()
    };

    dict_types_model::Model::create_dict_type(db, Some(tenant_id), am, user_id)
        .await
        .model_err()
}

/// Create a type override with explicit status (bypasses `create_dict_type` which forces "active").
/// Used when tenant toggles a system type to disabled — needs override row with disabled status.
async fn create_type_override_with_status(
    db: &DatabaseConnection,
    system_row: &dict_types::Model,
    tenant_id: Uuid,
    user_id: Uuid,
    status: &str,
) -> loco_rs::Result<dict_types::Model> {
    let am = dict_types_model::ActiveModel {
        code: ActiveValue::Set(system_row.code.clone()),
        name: ActiveValue::Set(system_row.name.clone()),
        source_type_id: ActiveValue::Set(Some(system_row.id)),
        description: ActiveValue::Set(system_row.description.clone()),
        tenant_id: ActiveValue::Set(Some(tenant_id)),
        version: ActiveValue::Set(1),
        status: ActiveValue::Set(status.to_string()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };
    // Insert directly — before_save hook generates the id
    am.insert(db).await.db_err()
}

/// Apply mutable field updates to a dict item.
async fn apply_dict_item_update(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
    params: &UpdateDictItemRequest,
) -> loco_rs::Result<dict_items::Model> {
    let mut am = dict_items_model::ActiveModel {
        version: ActiveValue::Set(params.version),
        ..Default::default()
    };
    if let Some(ref name) = params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(parent_id) = params.parent_id {
        am.parent_id = ActiveValue::Set(parent_id);
    }
    if let Some(sort_order) = params.sort_order {
        am.sort_order = ActiveValue::Set(sort_order);
    }
    if let Some(ref description) = params.description {
        am.description = ActiveValue::Set(description.clone());
    }

    dict_items_model::Model::update_with_version(db, id, am, user_id)
        .await
        .model_err()
}

/// Create a dict item override from a system row (copy-on-write).
/// If an override already exists for this tenant+source, update it instead.
async fn create_item_override(
    db: &DatabaseConnection,
    system_item: &dict_items::Model,
    tenant_id: Uuid,
    user_id: Uuid,
    params: Option<&UpdateDictItemRequest>,
) -> loco_rs::Result<dict_items::Model> {
    // Check if override already exists
    if let Ok(existing_override) =
        dict_items_model::Model::find_override_by_tenant_and_source(
            db,
            tenant_id,
            system_item.id,
        )
        .await
    {
        if let Some(p) = params {
            return apply_dict_item_update(db, existing_override.id, user_id, p).await;
        }
        return Ok(existing_override);
    }

    let name = params
        .and_then(|p| p.name.as_ref())
        .unwrap_or(&system_item.name);
    let parent_id = params
        .and_then(|p| p.parent_id)
        .unwrap_or(system_item.parent_id);
    let sort_order = params
        .and_then(|p| p.sort_order)
        .unwrap_or(system_item.sort_order);
    let description = params
        .and_then(|p| p.description.as_ref())
        .unwrap_or(&system_item.description);

    let am = dict_items_model::ActiveModel {
        dict_type_id: ActiveValue::Set(system_item.dict_type_id),
        code: ActiveValue::Set(system_item.code.clone()),
        name: ActiveValue::Set(name.clone()),
        value: ActiveValue::Set(system_item.value.clone()),
        parent_id: ActiveValue::Set(parent_id),
        sort_order: ActiveValue::Set(sort_order),
        source_item_id: ActiveValue::Set(Some(system_item.id)),
        description: ActiveValue::Set(description.clone()),
        ..Default::default()
    };

    dict_items_model::Model::create_dict_item(db, Some(tenant_id), am, user_id)
        .await
        .model_err()
}

/// Create item override with explicit status (bypasses `create_dict_item` which forces "active").
/// Used when tenant toggles a system item to disabled.
async fn create_item_override_with_status(
    db: &DatabaseConnection,
    system_item: &dict_items::Model,
    tenant_id: Uuid,
    user_id: Uuid,
    status: &str,
) -> loco_rs::Result<dict_items::Model> {
    let am = dict_items_model::ActiveModel {
        dict_type_id: ActiveValue::Set(system_item.dict_type_id),
        code: ActiveValue::Set(system_item.code.clone()),
        name: ActiveValue::Set(system_item.name.clone()),
        value: ActiveValue::Set(system_item.value.clone()),
        parent_id: ActiveValue::Set(system_item.parent_id),
        sort_order: ActiveValue::Set(system_item.sort_order),
        source_item_id: ActiveValue::Set(Some(system_item.id)),
        description: ActiveValue::Set(system_item.description.clone()),
        tenant_id: ActiveValue::Set(Some(tenant_id)),
        version: ActiveValue::Set(1),
        status: ActiveValue::Set(status.to_string()),
        updated_by: ActiveValue::Set(Some(user_id)),
        ..Default::default()
    };
    // Insert directly — before_save hook generates the id
    am.insert(db).await.db_err()
}

// ══════════════════════════════════════════════
//  Tree building
// ══════════════════════════════════════════════

/// Build tree from plain models (super admin view — system items only).
/// Sorted by `sort_order` in memory.
fn build_model_tree(
    all_items: &[dict_items::Model],
    parent_id: Option<Uuid>,
) -> Vec<DictItemTreeResponse> {
    let mut children: Vec<_> = all_items
        .iter()
        .filter(|item| item.parent_id == parent_id)
        .collect();
    children.sort_by_key(|item| item.sort_order);

    children
        .iter()
        .map(|item| {
            let sub = build_model_tree(all_items, Some(item.id));
            DictItemTreeResponse::from_model(item, sub)
        })
        .collect()
}

/// Build tree from effective items (tenant view).
/// Uses `base_item_id` for parent-child matching: `parent_id` references the system item id,
/// and `base_item_id` = `COALESCE(source_item_id`, id) normalizes identities.
fn build_effective_tree(
    all_items: &[EffectiveDictItem],
    parent_base_id: Option<Uuid>,
) -> Vec<DictItemTreeResponse> {
    let mut children: Vec<_> = all_items
        .iter()
        .filter(|item| item.parent_id == parent_base_id)
        .collect();
    children.sort_by_key(|item| item.sort_order);

    children
        .iter()
        .map(|item| {
            let sub = build_effective_tree(all_items, Some(item.base_item_id));
            DictItemTreeResponse::from_effective(item, sub)
        })
        .collect()
}
