use loco_openapi::prelude::*;
use loco_rs::prelude::*;
use std::sync::OnceLock;

use crate::extractors::TenantContext;
use crate::services::casbin_service::SharedEnforcer;
use crate::services::permission_service;
use crate::views::errors::parse_uuid;
use crate::views::pagination::PaginationParams;
use crate::views::permissions::{
    PermissionResponse, SyncPermissionsRequest, UpdatePermissionRequest,
};

/// Cached `OpenAPI` spec used by metadata-enriched permission endpoints.
///
/// At **runtime** the loco-openapi initializer stores the merged spec in a
/// global `OPENAPI_SPEC`; we read it via `get_openapi_spec()`.
///
/// In the **test** environment the initializer is skipped, so we build the spec
/// once from a clean set of route registrations.
static OPENAPI_SPEC_CACHE: OnceLock<utoipa::openapi::OpenApi> = OnceLock::new();

fn init_openapi_spec() -> &'static utoipa::openapi::OpenApi {
    OPENAPI_SPEC_CACHE.get_or_init(|| {
        // Runtime: initializer has set the spec.
        if let Ok(spec) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            loco_openapi::utils::get_openapi_spec().clone()
        })) {
            return spec;
        }

        // Test environment fallback:
        // Clear accumulated duplicate routes from prior test boots, then
        // re-register a single clean set and merge.
        loco_openapi::openapi::clear_routes();
        register_all_controller_routes();
        let (_, spec) = loco_openapi::openapi::get_merged_router().split_for_parts();
        spec
    })
}

/// Calls every controller's `routes()` function once.
/// The `openapi()` wrappers inside each function push to the global
/// `OPENAPI_ROUTES`, giving us a single clean set of route registrations.
fn register_all_controller_routes() {
    use crate::controllers;
    let _ = controllers::auth::routes();
    let _ = controllers::roles::routes();
    let _ = controllers::roles::user_role_routes();
    let _ = controllers::permissions::routes();
    let _ = controllers::sys_menus::routes();
    let _ = controllers::menus::routes();
    let _ = controllers::menus::user_menu_routes();
    let _ = controllers::dicts::dict_type_routes();
    let _ = controllers::dicts::dict_item_routes();
    let _ = controllers::users::routes();
    let _ = controllers::tenants::routes();
    let _ = controllers::tenants::sys_routes();
    let _ = controllers::role_templates::routes();
}

/// Public accessor for the cached `OpenAPI` spec.
/// Used by other controllers (e.g. `roles::assignable_permissions`) to get
/// the spec for building metadata-enriched permission responses.
#[must_use]
pub fn get_openapi_spec_ref() -> &'static utoipa::openapi::OpenApi {
    init_openapi_spec()
}

#[utoipa::path(
    get,
    path = "/api/permissions",
    tag = "权限管理",
    description = "查询权限列表",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn list(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Response> {
    let pagination = pagination.into();
    let result = permission_service::list_permissions(&ctx.db, &pagination).await?;

    format::json(result)
}

#[utoipa::path(
    put,
    path = "/api/permissions/{id}",
    tag = "权限管理",
    description = "更新权限",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn update(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
    Json(params): Json<UpdatePermissionRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let id_uuid = parse_uuid(id)?;
    let permission =
        permission_service::update_permission(&ctx.db, id_uuid, tc.user_id, &params)
            .await?;

    format::json(PermissionResponse::from_model(&permission))
}

#[utoipa::path(
    delete,
    path = "/api/permissions/{id}",
    tag = "权限管理",
    description = "删除权限",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn remove(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Path(id): Path<String>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }
    let id_uuid = parse_uuid(id)?;
    let Some(enforcer) = ctx.shared_store.get::<SharedEnforcer>() else {
        return crate::views::errors::internal(
            "casbin.not_initialized",
            "Casbin 策略引擎未初始化",
        );
    };

    permission_service::delete_permission(&ctx.db, &enforcer, id_uuid).await?;

    format::json(())
}

#[utoipa::path(
    get,
    path = "/api/permissions/with-metadata",
    tag = "权限管理",
    description = "查询所有权限（含路由元数据tag/description）及未匹配的路由",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn with_metadata(
    _tc: TenantContext,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let openapi = get_openapi_spec_ref();
    let result =
        permission_service::get_permissions_with_metadata_and_unmatched(&ctx.db, openapi)
            .await?;

    format::json(result)
}

#[utoipa::path(
    post,
    path = "/api/permissions/sync",
    tag = "权限管理",
    description = "批量同步权限（仅超级管理员）",
    responses((status = 200, description = "Success"))
)]
#[debug_handler]
pub(crate) async fn sync(
    tc: TenantContext,
    State(ctx): State<AppContext>,
    Json(params): Json<SyncPermissionsRequest>,
) -> Result<Response> {
    if !tc.is_super_admin {
        return crate::views::errors::authz::super_admin_required();
    }

    let created =
        permission_service::sync_permissions(&ctx.db, tc.user_id, &params.items).await?;

    let results: Vec<PermissionResponse> =
        created.iter().map(PermissionResponse::from_model).collect();

    format::json(results)
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/permissions")
        .add("/", openapi(get(list), routes!(list)))
        .add(
            "/with-metadata",
            openapi(get(with_metadata), routes!(with_metadata)),
        )
        .add("/sync", openapi(post(sync), routes!(sync)))
        .add("/{id}", openapi(put(update), routes!(update)))
        .add("/{id}", openapi(delete(remove), routes!(remove)))
}
