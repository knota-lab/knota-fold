use crate::utils::error::IntoModelResult;
use chrono::Utc;
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter,
};
use uuid::Uuid;

use crate::extractors::TenantContext;
use crate::models::_entities::api_keys as api_keys_entity;
use crate::models::{api_keys, roles};
use crate::services::casbin_service::{
    remove_api_key_role, sync_api_key_role, SharedEnforcer,
};
use crate::utils::error::OptionErrInto;
use crate::views::api_keys::{
    ApiKeyResponse, ChangeApiKeyRoleRequest, UpdateApiKeyRequest,
};
use crate::views::errors::{parse_uuid, CodedErrorResponse};

async fn load_role_name(
    db: &DatabaseConnection,
    role_id: Uuid,
    tenant_id: Uuid,
) -> Result<String> {
    Ok(roles::Model::find_by_id_and_tenant(db, role_id, tenant_id)
        .await?
        .name)
}

#[utoipa::path(
    get,
    path = "/api/api-keys",
    tag = "API Key",
    description = "查询 API Key 列表",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Tenant-scoped API Keys without plaintext secrets", body = [ApiKeyResponse]),
        (status = 401, description = "Invalid JWT", body = CodedErrorResponse),
        (status = 403, description = "Role permission denied", body = CodedErrorResponse),
        (status = 500, description = "Internal error", body = CodedErrorResponse)
    )
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let items = api_keys::Entity::find()
        .filter(api_keys_entity::Column::TenantId.eq(tc.tenant_id))
        .all(&ctx.db)
        .await?;

    let mut responses = Vec::with_capacity(items.len());
    for item in items {
        let role_name = load_role_name(&ctx.db, item.role_id, tc.tenant_id).await?;
        responses.push(ApiKeyResponse::from_model(&item, role_name));
    }

    format::json(responses)
}

#[utoipa::path(
    get,
    path = "/api/api-keys/{id}",
    tag = "API Key",
    description = "查询 API Key 详情",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "API Key UUID")),
    responses(
        (status = 200, description = "API Key metadata without plaintext secret", body = ApiKeyResponse),
        (status = 400, description = "Invalid UUID", body = CodedErrorResponse),
        (status = 401, description = "Invalid JWT", body = CodedErrorResponse),
        (status = 403, description = "Role permission denied", body = CodedErrorResponse),
        (status = 404, description = "API Key not found in current tenant", body = CodedErrorResponse),
        (status = 500, description = "Internal error", body = CodedErrorResponse)
    )
)]
#[debug_handler]
pub(crate) async fn detail(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id = parse_uuid(id)?;
    let item = api_keys::Model::find_by_id_and_tenant(&ctx.db, id, tc.tenant_id)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;
    let role_name = load_role_name(&ctx.db, item.role_id, tc.tenant_id).await?;
    format::json(ApiKeyResponse::from_model(&item, role_name))
}

#[utoipa::path(
    put,
    path = "/api/api-keys/{id}",
    tag = "API Key",
    description = "更新 API Key",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "API Key UUID")),
    request_body = UpdateApiKeyRequest,
    responses(
        (status = 200, description = "Updated", body = ApiKeyResponse),
        (status = 400, description = "Invalid UUID or request", body = CodedErrorResponse),
        (status = 401, description = "Invalid JWT", body = CodedErrorResponse),
        (status = 403, description = "Role permission denied", body = CodedErrorResponse),
        (status = 404, description = "API Key not found in current tenant", body = CodedErrorResponse),
        (status = 500, description = "Internal error", body = CodedErrorResponse)
    )
)]
#[debug_handler]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdateApiKeyRequest>,
) -> Result<Response> {
    let id = parse_uuid(id)?;
    let item = api_keys::Model::find_by_id_and_tenant(&ctx.db, id, tc.tenant_id)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let mut active_model: api_keys::ActiveModel = item.into();
    if let Some(name) = params.name {
        active_model.name = ActiveValue::Set(name);
    }
    if let Some(description) = params.description {
        active_model.description = ActiveValue::Set(description);
    }

    let updated = active_model.update(&ctx.db).await.model_err()?;
    let role_name = load_role_name(&ctx.db, updated.role_id, tc.tenant_id).await?;
    format::json(ApiKeyResponse::from_model(&updated, role_name))
}

#[utoipa::path(
    post,
    path = "/api/api-keys/{id}/revoke",
    tag = "API Key",
    description = "吊销 API Key",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "API Key UUID")),
    responses(
        (status = 200, description = "Revoked", body = ApiKeyResponse),
        (status = 400, description = "Invalid UUID", body = CodedErrorResponse),
        (status = 401, description = "Invalid JWT", body = CodedErrorResponse),
        (status = 403, description = "Role permission denied", body = CodedErrorResponse),
        (status = 404, description = "API Key not found in current tenant", body = CodedErrorResponse),
        (status = 500, description = "Policy synchronization or internal error", body = CodedErrorResponse)
    )
)]
#[debug_handler]
pub(crate) async fn revoke(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id = parse_uuid(id)?;
    let item = api_keys::Model::find_by_id_and_tenant(&ctx.db, id, tc.tenant_id)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let mut active_model: api_keys::ActiveModel = item.into();
    active_model.revoked_at = ActiveValue::Set(Some(Utc::now().fixed_offset()));
    let updated = active_model.update(&ctx.db).await.model_err()?;

    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };
    remove_api_key_role(
        &enforcer,
        &updated.id.to_string(),
        &tc.tenant_id.to_string(),
    )
    .await
    .map_err(|e| {
        Error::CustomError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            loco_rs::controller::ErrorDetail::new(
                "casbin.sync_failed",
                &format!("Casbin 策略同步失败: {e}"),
            ),
        )
    })?;

    let role_name = load_role_name(&ctx.db, updated.role_id, tc.tenant_id).await?;
    format::json(ApiKeyResponse::from_model(&updated, role_name))
}

#[utoipa::path(
    put,
    path = "/api/api-keys/{id}/role",
    tag = "API Key",
    description = "切换 API Key 角色",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "API Key UUID")),
    request_body = ChangeApiKeyRoleRequest,
    responses(
        (status = 200, description = "Role changed", body = ApiKeyResponse),
        (status = 400, description = "Invalid API Key or role UUID", body = CodedErrorResponse),
        (status = 401, description = "Invalid JWT", body = CodedErrorResponse),
        (status = 403, description = "Role permission denied", body = CodedErrorResponse),
        (status = 404, description = "API Key or role not found in current tenant", body = CodedErrorResponse),
        (status = 500, description = "Policy synchronization or internal error", body = CodedErrorResponse)
    )
)]
#[debug_handler]
pub(crate) async fn change_role(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<ChangeApiKeyRoleRequest>,
) -> Result<Response> {
    let id = parse_uuid(id)?;
    let role_id = parse_uuid(params.role_id)?;

    let role =
        roles::Model::find_by_id_and_tenant(&ctx.db, role_id, tc.tenant_id).await?;
    let item = api_keys::Model::find_by_id_and_tenant(&ctx.db, id, tc.tenant_id)
        .await?
        .or_err(crate::error_info::common::NOT_FOUND)?;

    let mut active_model: api_keys::ActiveModel = item.into();
    active_model.role_id = ActiveValue::Set(role.id);
    let updated = active_model.update(&ctx.db).await.model_err()?;

    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };
    sync_api_key_role(
        &enforcer,
        &updated.id.to_string(),
        &role.code,
        &tc.tenant_id.to_string(),
    )
    .await
    .map_err(|e| {
        Error::CustomError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            loco_rs::controller::ErrorDetail::new(
                "casbin.sync_failed",
                &format!("Casbin 策略同步失败: {e}"),
            ),
        )
    })?;

    format::json(ApiKeyResponse::from_model(&updated, role.name))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/api-keys")
        .add("/", openapi(get(list), routes!(list)))
        .add("/{id}", openapi(get(detail), routes!(detail)))
        .add("/{id}", openapi(put(update), routes!(update)))
        .add("/{id}/revoke", openapi(post(revoke), routes!(revoke)))
        .add(
            "/{id}/role",
            openapi(put(change_role), routes!(change_role)),
        )
}
