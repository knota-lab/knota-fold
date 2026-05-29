use axum::http::StatusCode;
use loco_rs::controller::ErrorDetail;
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter,
};
use uuid::Uuid;

use crate::models::_entities::{role_menus, sys_menus};
use crate::models::sys_menus as sys_menus_model;
use crate::services::audit_service;
use crate::utils::error::{IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext, SysMenuAuditSnapshot};
use crate::views::sys_menus::{
    CreateSysMenuRequest, SysMenuResponse, SysMenuTreeResponse, UpdateSysMenuRequest,
};

const MAX_MENU_DEPTH: usize = 20;

fn build_tree(
    all_menus: &[sys_menus::Model],
    parent_id: Option<Uuid>,
    depth: usize,
) -> Vec<SysMenuTreeResponse> {
    if depth >= MAX_MENU_DEPTH {
        return vec![];
    }

    let mut nodes: Vec<&sys_menus::Model> = all_menus
        .iter()
        .filter(|menu| menu.parent_id == parent_id)
        .collect();

    nodes.sort_by_key(|menu| menu.sort_order);

    nodes
        .into_iter()
        .map(|menu| {
            let children = build_tree(all_menus, Some(menu.id), depth + 1);
            SysMenuTreeResponse::from_model(menu, children)
        })
        .collect()
}

#[tracing::instrument(skip_all)]
pub async fn list_sys_menus(
    db: &DatabaseConnection,
) -> loco_rs::Result<Vec<SysMenuResponse>> {
    let menus = sys_menus::Model::find_all_not_deleted(db).await?;

    Ok(menus.iter().map(SysMenuResponse::from_model).collect())
}

#[tracing::instrument(skip_all)]
pub async fn get_sys_menu_tree(
    db: &DatabaseConnection,
) -> loco_rs::Result<Vec<SysMenuTreeResponse>> {
    let menus = sys_menus::Model::find_all_not_deleted(db).await?;

    Ok(build_tree(&menus, None, 0))
}

#[tracing::instrument(skip_all)]
pub async fn create_sys_menu(
    db: &DatabaseConnection,
    user_id: Uuid,
    params: &CreateSysMenuRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<sys_menus::Model> {
    let mut am = sys_menus_model::ActiveModel {
        name: ActiveValue::Set(params.name.clone()),
        code: ActiveValue::Set(params.code.clone()),
        menu_type: ActiveValue::Set(params.menu_type.clone()),
        ..Default::default()
    };

    if let Some(ref path) = params.path {
        am.path = ActiveValue::Set(Some(path.clone()));
    }
    if let Some(ref alias) = params.alias {
        am.alias = ActiveValue::Set(Some(alias.clone()));
    }
    if let Some(ref icon) = params.icon {
        am.icon = ActiveValue::Set(Some(icon.clone()));
    }
    if let Some(parent_id) = params.parent_id {
        am.parent_id = ActiveValue::Set(Some(parent_id));
    }
    if let Some(is_cache) = params.is_cache {
        am.is_cache = ActiveValue::Set(is_cache);
    }
    if let Some(sort_order) = params.sort_order {
        am.sort_order = ActiveValue::Set(sort_order);
    }
    if let Some(ref remark) = params.remark {
        am.remark = ActiveValue::Set(Some(remark.clone()));
    }

    let menu = sys_menus::Model::create_menu(db, am, user_id)
        .await
        .model_err()?;
    let snapshot = SysMenuAuditSnapshot::from(&menu);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "sys_menu",
        &menu.id.to_string(),
        None::<&SysMenuAuditSnapshot>,
        Some(&snapshot),
    )
    .await?;
    Ok(menu)
}

#[tracing::instrument(skip_all)]
pub async fn update_sys_menu(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
    params: &UpdateSysMenuRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<sys_menus::Model> {
    let existing = sys_menus::Entity::find_by_id(id)
        .one(db)
        .await
        .model_err()?
        .ok_or_else(|| {
            loco_rs::Error::CustomError(
                StatusCode::NOT_FOUND,
                ErrorDetail::new("common.not_found", "菜单未找到"),
            )
        })?;
    let before = SysMenuAuditSnapshot::from(&existing);

    let mut am = sys_menus_model::ActiveModel {
        version: ActiveValue::Set(params.version),
        ..Default::default()
    };

    if let Some(ref name) = params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(ref code) = params.code {
        am.code = ActiveValue::Set(code.clone());
    }
    if let Some(ref menu_type) = params.menu_type {
        am.menu_type = ActiveValue::Set(menu_type.clone());
    }
    if let Some(ref path) = params.path {
        am.path = ActiveValue::Set(path.clone());
    }
    if let Some(ref alias) = params.alias {
        am.alias = ActiveValue::Set(alias.clone());
    }
    if let Some(ref icon) = params.icon {
        am.icon = ActiveValue::Set(icon.clone());
    }
    if let Some(parent_id) = params.parent_id {
        am.parent_id = ActiveValue::Set(parent_id);
    }
    if let Some(is_cache) = params.is_cache {
        am.is_cache = ActiveValue::Set(is_cache);
    }
    if let Some(sort_order) = params.sort_order {
        am.sort_order = ActiveValue::Set(sort_order);
    }
    if let Some(ref remark) = params.remark {
        am.remark = ActiveValue::Set(remark.clone());
    }
    if let Some(ref status) = params.status {
        am.status = ActiveValue::Set(status.clone());
    }

    let menu = sys_menus::Model::update_with_version(db, id, am, user_id)
        .await
        .model_err()?;
    let after = SysMenuAuditSnapshot::from(&menu);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "sys_menu",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;
    Ok(menu)
}

#[tracing::instrument(skip_all)]
pub async fn delete_sys_menu(
    db: &DatabaseConnection,
    id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = sys_menus::Entity::find_by_id(id)
        .one(db)
        .await
        .model_err()?
        .or_err(crate::error_info::common::NOT_FOUND)?;
    let before = SysMenuAuditSnapshot::from(&existing);

    // Check for active children
    let children_count = sys_menus::Entity::find()
        .filter(sys_menus::Column::ParentId.eq(id))
        .filter(sys_menus::Column::DeletedAt.is_null())
        .count(db)
        .await
        .model_err()?;
    if children_count > 0 {
        return Err(loco_rs::Error::CustomError(
            StatusCode::CONFLICT,
            ErrorDetail::new(
                "sys_menu.has_active_children",
                "菜单存在活跃子菜单，无法删除",
            ),
        ));
    }

    // Clean role_menus references
    role_menus::Entity::delete_many()
        .filter(role_menus::Column::SysMenuId.eq(id))
        .exec(db)
        .await
        .model_err()?;

    sys_menus::Model::soft_delete(db, id).await?;
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Delete,
        "sys_menu",
        &id.to_string(),
        Some(&before),
        None::<&SysMenuAuditSnapshot>,
    )
    .await?;
    Ok(())
}
