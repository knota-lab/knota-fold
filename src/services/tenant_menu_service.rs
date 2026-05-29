use sea_orm::{ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::models::_entities::{
    role_menus, roles, sys_menus, tenant_menu_overrides, user_roles,
};
use crate::models::tenant_menu_overrides as overrides_model;
use crate::services::audit_service;
use crate::utils::error::IntoModelResult;
use crate::views::audit_logs::{
    AuditAction, AuditContext, TenantMenuOverrideAuditSnapshot,
};
use crate::views::menus::{MergedMenuTreeResponse, UpdateOverrideRequest};
use crate::views::roles::AssignableMenusResponse;

#[derive(Clone)]
struct MergedMenuItem {
    id: Uuid,
    parent_id: Option<Uuid>,
    code: String,
    name: String,
    path: Option<String>,
    alias: Option<String>,
    icon: Option<String>,
    menu_type: String,
    is_cache: bool,
    sort_order: i32,
    is_hidden: bool,
}

impl MergedMenuItem {
    fn from_parts(
        menu: &sys_menus::Model,
        override_record: Option<&tenant_menu_overrides::Model>,
    ) -> Self {
        Self {
            id: menu.id,
            parent_id: menu.parent_id,
            code: menu.code.clone(),
            name: override_record
                .and_then(|record| record.custom_name.clone())
                .unwrap_or_else(|| menu.name.clone()),
            path: menu.path.clone(),
            alias: menu.alias.clone(),
            icon: override_record
                .and_then(|record| record.custom_icon.clone())
                .or_else(|| menu.icon.clone()),
            menu_type: menu.menu_type.clone(),
            is_cache: menu.is_cache,
            sort_order: override_record
                .and_then(|record| record.custom_sort)
                .unwrap_or(menu.sort_order),
            is_hidden: override_record
                .map(|record| record.is_hidden)
                .unwrap_or(false),
        }
    }

    fn into_tree_response(
        self,
        children: Vec<MergedMenuTreeResponse>,
    ) -> MergedMenuTreeResponse {
        MergedMenuTreeResponse {
            id: self.id.to_string(),
            parent_id: self.parent_id.map(|parent_id| parent_id.to_string()),
            code: self.code,
            name: self.name,
            path: self.path,
            alias: self.alias,
            icon: self.icon,
            menu_type: self.menu_type,
            is_cache: self.is_cache,
            sort_order: self.sort_order,
            children,
        }
    }
}

const MAX_MENU_DEPTH: usize = 20;

fn build_tree(
    all_menus: &[MergedMenuItem],
    parent_id: Option<Uuid>,
    depth: usize,
) -> Vec<MergedMenuTreeResponse> {
    if depth >= MAX_MENU_DEPTH {
        return vec![];
    }

    let mut nodes: Vec<MergedMenuItem> = all_menus
        .iter()
        .filter(|menu| menu.parent_id == parent_id)
        .cloned()
        .collect();

    nodes.sort_by_key(|menu| menu.sort_order);

    nodes
        .into_iter()
        .map(|menu| {
            let children = build_tree(all_menus, Some(menu.id), depth + 1);
            menu.into_tree_response(children)
        })
        .collect()
}

fn merge_menus(
    menus: &[sys_menus::Model],
    overrides_map: &HashMap<Uuid, &tenant_menu_overrides::Model>,
) -> Vec<MergedMenuItem> {
    menus
        .iter()
        .map(|menu| {
            MergedMenuItem::from_parts(menu, overrides_map.get(&menu.id).copied())
        })
        .collect()
}

/// Merge sys_menus with tenant_menu_overrides in memory — implements §1.4 SQL logic
#[tracing::instrument(skip_all)]
pub async fn get_merged_menu_tree(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> loco_rs::Result<Vec<MergedMenuTreeResponse>> {
    let menus = sys_menus::Model::find_active(db).await?;
    let overrides = tenant_menu_overrides::Model::find_by_tenant(db, tenant_id).await?;
    let overrides_map: HashMap<Uuid, &tenant_menu_overrides::Model> = overrides
        .iter()
        .map(|record| (record.sys_menu_id, record))
        .collect();
    let merged = merge_menus(&menus, &overrides_map);

    Ok(build_tree(&merged, None, 0))
}

/// Compute the union of all menu IDs across all the user's roles in a given tenant.
/// Used for delegation checks: a user can only assign menus they themselves hold.
#[tracing::instrument(skip_all)]
pub async fn get_user_effective_menu_ids(
    db: &DatabaseConnection,
    user_id: Uuid,
    tenant_id: Uuid,
) -> loco_rs::Result<HashSet<Uuid>> {
    let role_ids: Vec<Uuid> = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|ur| ur.role_id)
        .collect();

    if role_ids.is_empty() {
        return Ok(HashSet::new());
    }

    // Filter to only active roles
    let role_ids: Vec<Uuid> = roles::Entity::find()
        .filter(roles::Column::Id.is_in(role_ids))
        .filter(roles::Column::Status.eq("active"))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|r| r.id)
        .collect();

    if role_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let menu_ids: HashSet<Uuid> = role_menus::Entity::find()
        .filter(role_menus::Column::RoleId.is_in(role_ids))
        .filter(role_menus::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|rm| rm.sys_menu_id)
        .collect();

    Ok(menu_ids)
}

/// Build assignable menu tree for a role assignment dialog.
/// Returns the menu tree scoped to the actor's permissions (or all menus for super admin),
/// plus the target role's currently assigned menu IDs.
#[tracing::instrument(skip_all)]
pub async fn get_assignable_menus(
    db: &DatabaseConnection,
    role_id: Uuid,
    tenant_id: Uuid,
    actor_user_id: Uuid,
    is_super_admin: bool,
) -> loco_rs::Result<AssignableMenusResponse> {
    let menus = sys_menus::Model::find_active(db).await?;
    let overrides = tenant_menu_overrides::Model::find_by_tenant(db, tenant_id).await?;
    let overrides_map: HashMap<Uuid, &tenant_menu_overrides::Model> = overrides
        .iter()
        .map(|record| (record.sys_menu_id, record))
        .collect();

    let tree = if is_super_admin {
        // Super admin sees all menus
        let merged = merge_menus(&menus, &overrides_map);
        build_tree(&merged, None, 0)
    } else {
        // Non-super-admin: filter to only menus they hold
        let actor_scope =
            get_user_effective_menu_ids(db, actor_user_id, tenant_id).await?;

        // Expand to include ancestor directories so tree structure stays valid
        let menu_map: HashMap<Uuid, &sys_menus::Model> =
            menus.iter().map(|m| (m.id, m)).collect();
        let mut visible_ids = actor_scope.clone();
        for id in &actor_scope {
            let mut current = menu_map.get(id).and_then(|m| m.parent_id);
            while let Some(pid) = current {
                if !visible_ids.insert(pid) {
                    break;
                }
                current = menu_map.get(&pid).and_then(|m| m.parent_id);
            }
        }

        let merged: Vec<MergedMenuItem> = menus
            .iter()
            .map(|menu| {
                MergedMenuItem::from_parts(menu, overrides_map.get(&menu.id).copied())
            })
            .filter(|menu| visible_ids.contains(&menu.id))
            .collect();
        build_tree(&merged, None, 0)
    };

    // Get the target role's currently assigned menu IDs
    let assigned: Vec<String> = role_menus::Entity::find()
        .filter(role_menus::Column::RoleId.eq(role_id))
        .filter(role_menus::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?
        .into_iter()
        .map(|rm| rm.sys_menu_id.to_string())
        .collect();

    Ok(AssignableMenusResponse {
        menus: tree,
        assigned_menu_ids: assigned,
    })
}
#[tracing::instrument(skip_all)]
pub async fn upsert_override(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    sys_menu_id: Uuid,
    user_id: Uuid,
    params: &UpdateOverrideRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = tenant_menu_overrides::Entity::find()
        .filter(tenant_menu_overrides::Column::TenantId.eq(tenant_id))
        .filter(tenant_menu_overrides::Column::SysMenuId.eq(sys_menu_id))
        .one(db)
        .await
        .model_err()?;
    let before = existing.as_ref().map(TenantMenuOverrideAuditSnapshot::from);

    let mut am = <overrides_model::ActiveModel as std::default::Default>::default();
    if let Some(ref name) = params.custom_name {
        am.custom_name = ActiveValue::Set(Some(name.clone()));
    }
    if let Some(ref icon) = params.custom_icon {
        am.custom_icon = ActiveValue::Set(Some(icon.clone()));
    }
    if let Some(sort) = params.custom_sort {
        am.custom_sort = ActiveValue::Set(Some(sort));
    }
    if let Some(is_hidden) = params.is_hidden {
        am.is_hidden = ActiveValue::Set(is_hidden);
    }

    overrides_model::Model::upsert(db, tenant_id, sys_menu_id, am, user_id).await?;

    let updated = tenant_menu_overrides::Entity::find()
        .filter(tenant_menu_overrides::Column::TenantId.eq(tenant_id))
        .filter(tenant_menu_overrides::Column::SysMenuId.eq(sys_menu_id))
        .one(db)
        .await
        .model_err()?;

    if let Some(ref record) = updated {
        let after = TenantMenuOverrideAuditSnapshot::from(record);
        let action = if existing.is_some() {
            AuditAction::Update
        } else {
            AuditAction::Create
        };
        audit_service::log(
            db,
            audit_ctx,
            action,
            "tenant_menu_override",
            &record.id.to_string(),
            before.as_ref(),
            Some(&after),
        )
        .await?;
    }

    Ok(())
}

/// Delete an override (revert to platform defaults)
#[tracing::instrument(skip_all)]
pub async fn delete_override(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    sys_menu_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<()> {
    let existing = tenant_menu_overrides::Entity::find()
        .filter(tenant_menu_overrides::Column::TenantId.eq(tenant_id))
        .filter(tenant_menu_overrides::Column::SysMenuId.eq(sys_menu_id))
        .one(db)
        .await
        .model_err()?;

    overrides_model::Model::delete_override(db, tenant_id, sys_menu_id).await?;

    if let Some(ref record) = existing {
        let before = TenantMenuOverrideAuditSnapshot::from(record);
        audit_service::log(
            db,
            audit_ctx,
            AuditAction::Delete,
            "tenant_menu_override",
            &record.id.to_string(),
            Some(&before),
            None::<&TenantMenuOverrideAuditSnapshot>,
        )
        .await?;
    }

    Ok(())
}

/// Get user menus with 4-layer filtering
#[tracing::instrument(skip_all)]
pub async fn get_user_menus(
    db: &DatabaseConnection,
    user_id: Uuid,
    tenant_id: Uuid,
    is_super_admin: bool,
) -> loco_rs::Result<Vec<MergedMenuTreeResponse>> {
    let menus = sys_menus::Model::find_active(db).await?;
    let overrides = tenant_menu_overrides::Model::find_by_tenant(db, tenant_id).await?;
    let overrides_map: HashMap<Uuid, &tenant_menu_overrides::Model> = overrides
        .iter()
        .map(|record| (record.sys_menu_id, record))
        .collect();

    if is_super_admin {
        let merged = merge_menus(&menus, &overrides_map);
        return Ok(build_tree(&merged, None, 0));
    }

    let user_role_records = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .model_err()?;
    let role_ids: Vec<Uuid> = user_role_records
        .into_iter()
        .map(|record| record.role_id)
        .collect();

    // Filter to only active roles
    let role_ids: Vec<Uuid> = if role_ids.is_empty() {
        vec![]
    } else {
        roles::Entity::find()
            .filter(roles::Column::Id.is_in(role_ids))
            .filter(roles::Column::Status.eq("active"))
            .all(db)
            .await
            .model_err()?
            .into_iter()
            .map(|r| r.id)
            .collect()
    };

    let role_menu_ids: HashSet<Uuid> = if role_ids.is_empty() {
        HashSet::new()
    } else {
        role_menus::Entity::find()
            .filter(role_menus::Column::TenantId.eq(tenant_id))
            .filter(role_menus::Column::RoleId.is_in(role_ids))
            .all(db)
            .await
            .model_err()?
            .into_iter()
            .map(|record| record.sys_menu_id)
            .collect()
    };

    // Expand role_menu_ids to include all ancestor directories so the tree structure is valid.
    // Without ancestors, child menus with a parent_id can't form a rooted tree.
    let menu_map: HashMap<Uuid, &sys_menus::Model> =
        menus.iter().map(|m| (m.id, m)).collect();
    let mut visible_ids: HashSet<Uuid> = role_menu_ids.clone();
    for id in &role_menu_ids {
        let mut current = menu_map.get(id).and_then(|m| m.parent_id);
        while let Some(pid) = current {
            if !visible_ids.insert(pid) {
                break; // already added this ancestor and its chain
            }
            current = menu_map.get(&pid).and_then(|m| m.parent_id);
        }
    }

    let merged: Vec<MergedMenuItem> = menus
        .iter()
        .map(|menu| {
            MergedMenuItem::from_parts(menu, overrides_map.get(&menu.id).copied())
        })
        .filter(|menu| !menu.is_hidden && visible_ids.contains(&menu.id))
        .collect();

    Ok(build_tree(&merged, None, 0))
}
