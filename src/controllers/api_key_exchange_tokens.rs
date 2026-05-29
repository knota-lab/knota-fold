use chrono::{Duration, Utc};
use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use sea_orm::{ActiveModelTrait, ActiveValue};
use uuid::Uuid;

use crate::extractors::TenantContext;
use crate::models::{api_key_exchange_tokens, api_keys, roles, tenants};
use crate::services::api_key_service::{
    api_key_settings, generate_exchange_token, GeneratedKey,
};
use crate::services::casbin_service::{sync_api_key_role, SharedEnforcer};
use crate::utils::error::OptionErrInto;
use crate::views::api_key_exchange_tokens::{
    CreateExchangeTokenRequest, CreateExchangeTokenResponse, ExchangeInfoQuery,
    ExchangeInfoResponse, ExchangeKeyResponse, ExchangeRequest, ExchangeTokenResponse,
};
use crate::views::errors::{err_bad_request, parse_uuid};

const INVALID_EXCHANGE_TOKEN_MESSAGE: &str = "无效或已过期的兑换令牌";

async fn load_role(
    db: &DatabaseConnection,
    role_id: Uuid,
    tenant_id: Uuid,
) -> Result<roles::Model> {
    roles::Model::find_by_id_and_tenant(db, role_id, tenant_id)
        .await
        .map_err(Into::into)
}

#[utoipa::path(
    get,
    path = "/api/api-key-exchange-tokens",
    tag = "API Key",
    description = "查询交换令牌列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let items =
        api_key_exchange_tokens::Model::find_by_tenant(&ctx.db, tc.tenant_id).await?;
    let mut responses = Vec::with_capacity(items.len());
    for item in items {
        let role = load_role(&ctx.db, item.role_id, tc.tenant_id).await?;
        responses.push(ExchangeTokenResponse::from_model(&item, role.name));
    }
    format::json(responses)
}

#[utoipa::path(
    post,
    path = "/api/api-key-exchange-tokens",
    tag = "API Key",
    description = "创建交换令牌",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn create(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateExchangeTokenRequest>,
) -> Result<Response> {
    let settings = api_key_settings(&ctx)?;
    let current =
        api_key_exchange_tokens::Model::count_valid_by_tenant(&ctx.db, tc.tenant_id)
            .await?;
    if current >= settings.max_exchange_tokens_per_tenant as u64 {
        tracing::warn!(%current, max_exchange_tokens_per_tenant=settings.max_exchange_tokens_per_tenant, "exchange token limit exceeded");
        return Err(err_bad_request(
            "api_key.exchange_token_limit_exceeded",
            "兑换令牌数量已达上限",
        ));
    }

    let role_id = parse_uuid(params.role_id)?;
    let role = load_role(&ctx.db, role_id, tc.tenant_id).await?;
    let generated = generate_exchange_token();
    let expires_at = match params.expires_at {
        Some(value) => chrono::DateTime::parse_from_rfc3339(&value).map_err(|e| {
            err_bad_request(
                "api_key.expires_at_invalid",
                format!("无效的 expiresAt: {e}"),
            )
        })?,
        None => {
            Utc::now().fixed_offset()
                + Duration::hours(settings.default_exchange_ttl_hours.unwrap_or(24) as i64)
        }
    };

    let api_key_expires_at = match params.api_key_expires_at {
        Some(value) => {
            Some(chrono::DateTime::parse_from_rfc3339(&value).map_err(|e| {
                err_bad_request(
                    "api_key.api_key_expires_at_invalid",
                    format!("无效的 apiKeyExpiresAt: {e}"),
                )
            })?)
        }
        None => None,
    };

    let model = api_key_exchange_tokens::ActiveModel {
        id: ActiveValue::Set(generated.id),
        tenant_id: ActiveValue::Set(tc.tenant_id),
        name: ActiveValue::Set(params.name.clone()),
        token_hash: ActiveValue::Set(GeneratedKey::hash_key(&generated.plain_key)),
        token_prefix: ActiveValue::Set(generated.prefix.clone()),
        role_id: ActiveValue::Set(role.id),
        description: ActiveValue::Set(params.description),
        expires_at: ActiveValue::Set(expires_at),
        api_key_expires_at: ActiveValue::Set(api_key_expires_at),
        max_usage: ActiveValue::Set(params.max_usage.unwrap_or(1)),
        used_count: ActiveValue::Set(0),
        created_by: ActiveValue::Set(tc.user_id),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await?;

    let exchange_url = settings
        .exchange_base_url
        .unwrap_or_else(|| "/api-keys/exchange".to_string());

    format::json(CreateExchangeTokenResponse {
        id: model.id.to_string(),
        name: model.name,
        exchange_token: generated.plain_key.clone(),
        exchange_url: format!("{exchange_url}?token={}", generated.plain_key),
        token_prefix: model.token_prefix,
        role_id: model.role_id.to_string(),
        role_name: role.name,
        expires_at: model.expires_at.to_rfc3339(),
        max_usage: model.max_usage,
        created_at: model.created_at.to_rfc3339(),
    })
}

#[utoipa::path(
    get,
    path = "/api/api-key-exchange-tokens/{id}",
    tag = "API Key",
    description = "查询交换令牌详情",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn detail(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    let id = parse_uuid(id)?;
    let item =
        api_key_exchange_tokens::Model::find_by_id_and_tenant(&ctx.db, id, tc.tenant_id)
            .await?
            .or_err(crate::error_info::common::NOT_FOUND)?;
    let role = load_role(&ctx.db, item.role_id, tc.tenant_id).await?;
    format::json(ExchangeTokenResponse::from_model(&item, role.name))
}

#[utoipa::path(
    get,
    path = "/api/public/api-keys/exchange-info",
    tag = "API Key",
    description = "兑换信息预览",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn get_exchange_info(
    State(ctx): State<AppContext>,
    Query(query): Query<ExchangeInfoQuery>,
) -> Result<Response> {
    let token_hash = GeneratedKey::hash_key(&query.token);
    let item = api_key_exchange_tokens::Model::find_by_hash(&ctx.db, &token_hash)
        .await?
        .ok_or_else(|| {
            err_bad_request(
                "api_key.exchange_token_invalid",
                INVALID_EXCHANGE_TOKEN_MESSAGE,
            )
        })?;
    let tenant = tenants::Model::find_by_id(&ctx.db, item.tenant_id).await?;
    let role = load_role(&ctx.db, item.role_id, item.tenant_id).await?;

    format::json(ExchangeInfoResponse {
        tenant_name: tenant.name,
        role_name: role.name,
        expires_at: item.expires_at.to_rfc3339(),
        already_used: item.used_count >= item.max_usage,
    })
}

#[utoipa::path(
    post,
    path = "/api/public/api-keys/exchange",
    tag = "API Key",
    description = "执行兑换并返回 API Key 明文",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn exchange_key(
    State(ctx): State<AppContext>,
    Json(params): Json<ExchangeRequest>,
) -> Result<Response> {
    let token_hash = GeneratedKey::hash_key(&params.exchange_token);
    let token = api_key_exchange_tokens::Model::find_by_hash(&ctx.db, &token_hash)
        .await?
        .ok_or_else(|| {
            err_bad_request(
                "api_key.exchange_token_invalid",
                INVALID_EXCHANGE_TOKEN_MESSAGE,
            )
        })?;
    if !token.is_valid() {
        return Err(err_bad_request(
            "api_key.exchange_token_invalid",
            INVALID_EXCHANGE_TOKEN_MESSAGE,
        ));
    }

    let settings = api_key_settings(&ctx)?;
    let active_keys =
        api_keys::Model::count_active_by_tenant(&ctx.db, token.tenant_id).await?;
    if active_keys >= settings.max_keys_per_tenant as u64 {
        return Err(err_bad_request(
            "api_key.api_key_limit_exceeded",
            "API Key 数量已达上限",
        ));
    }

    let tenant = tenants::Model::find_by_id(&ctx.db, token.tenant_id).await?;
    if tenant.status != "active" {
        return Err(err_bad_request(
            "api_key.exchange_token_invalid",
            INVALID_EXCHANGE_TOKEN_MESSAGE,
        ));
    }

    let role = load_role(&ctx.db, token.role_id, token.tenant_id).await?;
    let generated =
        GeneratedKey::generate_with_bytes(&settings.env_prefix, settings.secret_bytes);

    let api_key = api_keys::ActiveModel {
        id: ActiveValue::Set(generated.id),
        tenant_id: ActiveValue::Set(token.tenant_id),
        name: ActiveValue::Set(token.name.clone()),
        key_prefix: ActiveValue::Set(generated.prefix.clone()),
        key_hash: ActiveValue::Set(GeneratedKey::hash_key(&generated.plain_key)),
        role_id: ActiveValue::Set(token.role_id),
        description: ActiveValue::Set(token.description.clone()),
        exchanged_from_id: ActiveValue::Set(Some(token.id)),
        expires_at: ActiveValue::Set(token.api_key_expires_at),
        revoked_at: ActiveValue::Set(None),
        last_used_at: ActiveValue::Set(None),
        created_by: ActiveValue::Set(token.created_by),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await?;

    api_key_exchange_tokens::Model::increment_usage(&ctx.db, token.id).await?;

    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };
    sync_api_key_role(
        &enforcer,
        &api_key.id.to_string(),
        &role.code,
        &api_key.tenant_id.to_string(),
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

    format::json(ExchangeKeyResponse {
        api_key_id: api_key.id.to_string(),
        api_key: generated.plain_key,
        key_prefix: api_key.key_prefix,
        role_name: role.name,
        expires_at: api_key.expires_at.map(|v| v.to_rfc3339()),
        created_at: api_key.created_at.to_rfc3339(),
    })
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/api-key-exchange-tokens")
        .add("/", openapi(get(list), routes!(list)))
        .add("/", openapi(post(create), routes!(create)))
        .add("/{id}", openapi(get(detail), routes!(detail)))
}

pub fn public_routes() -> Routes {
    Routes::new()
        .prefix("/api/public/api-keys")
        .add(
            "/exchange-info",
            openapi(get(get_exchange_info), routes!(get_exchange_info)),
        )
        .add(
            "/exchange",
            openapi(post(exchange_key), routes!(exchange_key)),
        )
}
