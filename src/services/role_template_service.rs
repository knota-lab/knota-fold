use loco_rs::prelude::*;
use sea_orm::{ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::models::_entities::sys_role_template_menus;
use crate::models::_entities::sys_role_template_permissions;
use crate::models::_entities::sys_role_templates;
use crate::models::sys_role_templates as templates_model;
use crate::services::audit_service;
use crate::utils::error::{IntoAppError, IntoModelResult};
use crate::views::audit_logs::{AuditAction, AuditContext, RoleTemplateAuditSnapshot};
use crate::views::role_templates::{
    CreateRoleTemplateRequest, RoleTemplateResponse, TemplatePermissionResponse,
    UpdateRoleTemplateRequest,
};

#[tracing::instrument(skip_all)]
pub async fn list_templates(
    db: &DatabaseConnection,
) -> loco_rs::Result<Vec<RoleTemplateResponse>> {
    let templates = templates_model::Model::find_all(db).await.model_err()?;

    Ok(templates
        .iter()
        .map(RoleTemplateResponse::from_model)
        .collect())
}

#[tracing::instrument(skip_all)]
pub async fn create_template(
    db: &DatabaseConnection,
    params: &CreateRoleTemplateRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<RoleTemplateResponse> {
    let am = templates_model::ActiveModel {
        code: ActiveValue::Set(params.code.clone()),
        name: ActiveValue::Set(params.name.clone()),
        description: ActiveValue::Set(params.description.clone()),
        is_default: ActiveValue::Set(params.is_default.unwrap_or(false)),
        sort_order: ActiveValue::Set(params.sort_order.unwrap_or(0)),
        ..Default::default()
    };

    let template = templates_model::Model::create(db, am).await.model_err()?;

    let snapshot = RoleTemplateAuditSnapshot::from(&template);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "role_template",
        &template.id.to_string(),
        None::<&RoleTemplateAuditSnapshot>,
        Some(&snapshot),
    )
    .await
    .model_err()?;

    Ok(RoleTemplateResponse::from_model(&template))
}

#[tracing::instrument(skip_all)]
pub async fn update_template(
    db: &DatabaseConnection,
    id: Uuid,
    params: &UpdateRoleTemplateRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<RoleTemplateResponse> {
    let existing = templates_model::Model::find_by_id(db, id)
        .await
        .model_err()?;
    let before = RoleTemplateAuditSnapshot::from(&existing);

    let mut am = templates_model::ActiveModel {
        ..Default::default()
    };

    if let Some(name) = &params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(description) = &params.description {
        am.description = ActiveValue::Set(description.clone());
    }
    if let Some(is_default) = params.is_default {
        am.is_default = ActiveValue::Set(is_default);
    }
    if let Some(sort_order) = params.sort_order {
        am.sort_order = ActiveValue::Set(sort_order);
    }

    let template = templates_model::Model::update_template(db, id, am)
        .await
        .model_err()?;

    let after = RoleTemplateAuditSnapshot::from(&template);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role_template",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await
    .model_err()?;

    Ok(RoleTemplateResponse::from_model(&template))
}

#[tracing::instrument(skip_all)]
pub async fn delete_template(
    db: &DatabaseConnection,
    id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing: sys_role_templates::Model = templates_model::Model::find_by_id(db, id)
        .await
        .model_err()?;
    let before = RoleTemplateAuditSnapshot::from(&existing);

    templates_model::Model::delete_template(db, id).await?;

    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Delete,
        "role_template",
        &id.to_string(),
        Some(&before),
        None::<&RoleTemplateAuditSnapshot>,
    )
    .await
    .model_err()?;

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn get_template_menu_ids(
    db: &DatabaseConnection,
    template_id: Uuid,
) -> loco_rs::Result<Vec<String>> {
    let records = sys_role_template_menus::Entity::find()
        .filter(sys_role_template_menus::Column::TemplateId.eq(template_id))
        .all(db)
        .await
        .db_err()?;

    Ok(records
        .into_iter()
        .map(|r| r.sys_menu_id.to_string())
        .collect())
}

#[tracing::instrument(skip_all)]
pub async fn sync_template_menus(
    db: &DatabaseConnection,
    template_id: Uuid,
    sys_menu_ids: Vec<Uuid>,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    // Verify template exists
    templates_model::Model::find_by_id(db, template_id)
        .await
        .model_err()?;

    let old_menu_ids: Vec<String> = sys_role_template_menus::Entity::find()
        .filter(sys_role_template_menus::Column::TemplateId.eq(template_id))
        .all(db)
        .await
        .db_err()?
        .into_iter()
        .map(|r| r.sys_menu_id.to_string())
        .collect();

    // Delete existing
    sys_role_template_menus::Entity::delete_many()
        .filter(sys_role_template_menus::Column::TemplateId.eq(template_id))
        .exec(db)
        .await
        .db_err()?;

    // Insert new
    for sys_menu_id in &sys_menu_ids {
        sys_role_template_menus::ActiveModel {
            template_id: ActiveValue::Set(template_id),
            sys_menu_id: ActiveValue::Set(*sys_menu_id),
        }
        .insert(db)
        .await
        .db_err()?;
    }

    let new_menu_ids: Vec<String> =
        sys_menu_ids.iter().map(|id| id.to_string()).collect();
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role_template_menus",
        &template_id.to_string(),
        Some(&serde_json::json!({"sysMenuIds": old_menu_ids})),
        Some(&serde_json::json!({"sysMenuIds": new_menu_ids})),
    )
    .await
    .model_err()?;

    Ok(())
}

#[tracing::instrument(skip_all)]
pub async fn get_template_permissions(
    db: &DatabaseConnection,
    template_id: Uuid,
) -> loco_rs::Result<Vec<TemplatePermissionResponse>> {
    let records = sys_role_template_permissions::Entity::find()
        .filter(sys_role_template_permissions::Column::TemplateId.eq(template_id))
        .all(db)
        .await
        .db_err()?;

    Ok(records
        .into_iter()
        .map(|r| TemplatePermissionResponse {
            obj: r.obj,
            act: r.act,
        })
        .collect())
}

#[tracing::instrument(skip_all)]
pub async fn sync_template_permissions(
    db: &DatabaseConnection,
    template_id: Uuid,
    permissions: Vec<(String, String)>,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    // Verify template exists
    templates_model::Model::find_by_id(db, template_id)
        .await
        .model_err()?;

    let old_permissions: Vec<serde_json::Value> =
        sys_role_template_permissions::Entity::find()
            .filter(sys_role_template_permissions::Column::TemplateId.eq(template_id))
            .all(db)
            .await
            .db_err()?
            .into_iter()
            .map(|r| serde_json::json!({"obj": r.obj, "act": r.act}))
            .collect();

    // Delete existing
    sys_role_template_permissions::Entity::delete_many()
        .filter(sys_role_template_permissions::Column::TemplateId.eq(template_id))
        .exec(db)
        .await
        .db_err()?;

    // Insert new
    for (obj, act) in &permissions {
        sys_role_template_permissions::ActiveModel {
            template_id: ActiveValue::Set(template_id),
            obj: ActiveValue::Set(obj.clone()),
            act: ActiveValue::Set(act.clone()),
        }
        .insert(db)
        .await
        .db_err()?;
    }

    let new_permissions: Vec<serde_json::Value> = permissions
        .iter()
        .map(|(obj, act)| serde_json::json!({"obj": obj, "act": act}))
        .collect();
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "role_template_permissions",
        &template_id.to_string(),
        Some(&serde_json::json!({"permissions": old_permissions})),
        Some(&serde_json::json!({"permissions": new_permissions})),
    )
    .await
    .model_err()?;

    Ok(())
}
