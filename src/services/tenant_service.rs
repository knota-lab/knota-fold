use axum::http::StatusCode;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::model::query;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter, TransactionTrait,
};
use uuid::Uuid;

use crate::models::_entities::role_menus;
use crate::models::_entities::role_permissions;
use crate::models::_entities::sys_role_template_menus;
use crate::models::_entities::sys_role_template_permissions;
use crate::models::_entities::sys_role_templates;
use crate::models::_entities::tenants;
use crate::models::permissions as permissions_model;
use crate::models::roles as roles_model;
use crate::models::tenants as tenants_model;
use crate::services::audit_service;
use crate::services::casbin_service::{self, SharedEnforcer};
use crate::utils::error::{IntoAppError, IntoModelResult, OptionErrInto};
use crate::views::audit_logs::{AuditAction, AuditContext, TenantAuditSnapshot};
use crate::views::errors::err_bad_request;
use crate::views::pagination::PaginatedResponse;
use crate::views::tenants::{
    CreateTenantRequest, TenantListParams, TenantResponse, UpdateTenantRequest,
};

#[tracing::instrument(skip_all)]
pub async fn list_tenants(
    db: &DatabaseConnection,
    pagination: &query::PaginationQuery,
    search: &TenantListParams,
) -> loco_rs::Result<PaginatedResponse<TenantResponse>> {
    let mut base_query = tenants::Entity::find();
    if let Some(ref name) = search.name {
        base_query = base_query.filter(tenants::Column::Name.contains(name));
    }
    if let Some(ref code) = search.code {
        base_query = base_query.filter(tenants::Column::Code.contains(code));
    }
    if let Some(ref status) = search.status {
        base_query = base_query.filter(tenants::Column::Status.eq(status));
    }
    let page_response = query::paginate(db, base_query, None, pagination).await?;

    Ok(PaginatedResponse::from_page_response(
        page_response,
        pagination,
        TenantResponse::from_model,
    ))
}

#[tracing::instrument(skip_all)]
pub async fn create_tenant(
    db: &DatabaseConnection,
    params: &CreateTenantRequest,
) -> Result<tenants::Model> {
    let am = tenants_model::ActiveModel {
        name: ActiveValue::Set(params.name.clone()),
        code: ActiveValue::Set(params.code.clone()),
        status: ActiveValue::Set(
            params
                .status
                .clone()
                .unwrap_or_else(|| "active".to_string()),
        ),
        description: ActiveValue::Set(params.description.clone()),
        ..std::default::Default::default()
    };

    tenants_model::Model::create(db, am).await.model_err()
}

/// Create a tenant with full initialization: roles from templates, menus, permissions, and Casbin policies.
#[tracing::instrument(skip_all)]
pub async fn create_tenant_with_init(
    db: &DatabaseConnection,
    enforcer: &SharedEnforcer,
    params: &CreateTenantRequest,
    operator_user_id: Uuid,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<tenants::Model> {
    let txn = db.begin().await.db_err()?;

    // Step 1: Create tenant in "initializing" status (will activate after Casbin sync)
    let am = tenants_model::ActiveModel {
        name: ActiveValue::Set(params.name.clone()),
        code: ActiveValue::Set(params.code.clone()),
        status: ActiveValue::Set("initializing".to_string()),
        description: ActiveValue::Set(params.description.clone()),
        ..std::default::Default::default()
    };
    let tenant = tenants_model::Model::create(&txn, am).await.model_err()?;

    // Step 2: Read all role templates
    let templates = sys_role_templates::Entity::find()
        .all(&txn)
        .await
        .db_err()?;

    // Collect Casbin policy data to sync AFTER commit (to avoid connection pool exhaustion)
    let mut casbin_role_policies: Vec<(String, Vec<(String, String)>)> = Vec::new();

    for template in &templates {
        // Step 3: Create role from template
        let role_am = roles_model::ActiveModel {
            name: ActiveValue::Set(template.name.clone()),
            code: ActiveValue::Set(template.code.clone()),
            description: ActiveValue::Set(template.description.clone()),
            is_system: ActiveValue::Set(true),
            ..Default::default()
        };
        let role =
            roles_model::Model::create_role(&txn, tenant.id, role_am, operator_user_id)
                .await
                .model_err()?;

        // Step 4: Copy template menu associations
        let template_menus = sys_role_template_menus::Entity::find()
            .filter(sys_role_template_menus::Column::TemplateId.eq(template.id))
            .all(&txn)
            .await
            .db_err()?;

        for tm in &template_menus {
            role_menus::ActiveModel {
                tenant_id: ActiveValue::Set(tenant.id),
                role_id: ActiveValue::Set(role.id),
                sys_menu_id: ActiveValue::Set(tm.sys_menu_id),
            }
            .insert(&txn)
            .await
            .db_err()?;
        }

        // Step 5: Copy template permission associations
        let template_perms = sys_role_template_permissions::Entity::find()
            .filter(sys_role_template_permissions::Column::TemplateId.eq(template.id))
            .all(&txn)
            .await
            .db_err()?;

        let mut obj_acts = Vec::new();
        for tp in &template_perms {
            let perm = permissions_model::Model::find_or_create_by_obj_act(
                &txn, &tp.obj, &tp.act,
            )
            .await
            .model_err()?;

            role_permissions::ActiveModel {
                tenant_id: ActiveValue::Set(tenant.id),
                role_id: ActiveValue::Set(role.id),
                permission_id: ActiveValue::Set(perm.id),
            }
            .insert(&txn)
            .await
            .db_err()?;

            obj_acts.push((tp.obj.clone(), tp.act.clone()));
        }

        casbin_role_policies.push((role.code.clone(), obj_acts));
    }

    txn.commit().await.db_err()?;

    // Step 6: Sync Casbin policies AFTER commit (avoids connection pool deadlock)
    for (role_code, obj_acts) in &casbin_role_policies {
        casbin_service::sync_role_permissions(
            enforcer,
            role_code,
            &tenant.id.to_string(),
            obj_acts,
        )
        .await
        .map_err(|e| {
            let desc = format!("Failed to sync casbin: {e}");
            Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("casbin.sync_failed", &desc),
            )
        })?;
    }

    // Step 7: Activate tenant now that all initialization is complete
    let activate_am = tenants_model::ActiveModel {
        status: ActiveValue::Set("active".to_string()),
        ..Default::default()
    };
    let tenant = tenants_model::Model::update(db, tenant.id, activate_am)
        .await
        .model_err()?;

    // Audit: record tenant creation (after full initialization + activation)
    let snapshot = TenantAuditSnapshot::from(&tenant);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Create,
        "tenant",
        &tenant.id.to_string(),
        None::<&TenantAuditSnapshot>,
        Some(&snapshot),
    )
    .await?;

    Ok(tenant)
}

#[tracing::instrument(skip_all)]
pub async fn update_tenant(
    db: &DatabaseConnection,
    id: Uuid,
    params: &UpdateTenantRequest,
    audit_ctx: &AuditContext,
) -> loco_rs::Result<tenants::Model> {
    // Guard: default tenant cannot be disabled
    if let Some(status) = &params.status {
        if status != "active" {
            let tenant = tenants::Entity::find_by_id(id)
                .one(db)
                .await
                .db_err()?
                .or_err(crate::error_info::common::NOT_FOUND)?;
            if tenant.code == "DEFAULT" {
                return Err(err_bad_request(
                    "tenant.default_cannot_disable",
                    "默认租户不允许被禁用",
                ));
            }
        }
    }

    // Load before state for audit
    let existing = tenants_model::Model::find_by_id(db, id).await.model_err()?;
    let before = TenantAuditSnapshot::from(&existing);

    let mut am = <tenants_model::ActiveModel as sea_orm::ActiveModelTrait>::default();

    if let Some(name) = &params.name {
        am.name = ActiveValue::Set(name.clone());
    }
    if let Some(status) = &params.status {
        am.status = ActiveValue::Set(status.clone());
    }
    if let Some(description) = &params.description {
        am.description = ActiveValue::Set(description.clone());
    }

    let tenant = tenants_model::Model::update(db, id, am).await.model_err()?;

    let after = TenantAuditSnapshot::from(&tenant);
    audit_service::log(
        db,
        audit_ctx,
        AuditAction::Update,
        "tenant",
        &id.to_string(),
        Some(&before),
        Some(&after),
    )
    .await?;

    Ok(tenant)
}

/// Find tenant by code, used by cross-tenant endpoints.
///
/// Distinguishes "tenant truly not found" (mapped to `Error::NotFound` so
/// callers can surface a 404) from real backend failures (kept as
/// `Error::Any`, surfacing as 5xx). This shape is required by Wave 2d
/// `resolve_target_tenant` so that DB outages are not silently masked as
/// `tenant_not_found` 404s.
#[tracing::instrument(skip_all)]
pub async fn find_tenant_by_code(
    db: &DatabaseConnection,
    code: &str,
) -> loco_rs::Result<tenants::Model> {
    match tenants_model::Model::find_by_code(db, code).await {
        Ok(tenant) => Ok(tenant),
        Err(ModelError::EntityNotFound) => Err(crate::views::errors::err_not_found(
            "tenant.not_found",
            "租户不存在",
        )),
        Err(e) => Err(Error::Any(e.into())),
    }
}
