use axum::http::StatusCode;
use loco_rs::app::AppContext;
use loco_rs::controller::ErrorDetail;

use crate::extractors::TenantContext;
use crate::services::casbin_service::{self, SharedEnforcer};

pub async fn check_permission(
    ctx: &AppContext,
    tc: &TenantContext,
    obj: &str,
    act: &str,
) -> loco_rs::Result<()> {
    if tc.is_super_admin {
        return Ok(());
    }

    let enforcer = ctx.shared_store.get::<SharedEnforcer>().ok_or_else(|| {
        loco_rs::Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.not_initialized", "Casbin 策略引擎未初始化"),
        )
    })?;

    let user_id_str = tc.user_id.to_string();
    let tenant_id_str = tc.tenant_id.to_string();

    let allowed =
        casbin_service::enforce_check(&enforcer, &user_id_str, &tenant_id_str, obj, act)
            .await
            .map_err(|e| {
                let desc = format!("Casbin enforcement error: {e}");
                loco_rs::Error::CustomError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorDetail::new("casbin.enforcement_error", &desc),
                )
            })?;

    if allowed {
        Ok(())
    } else {
        let desc = format!("权限不足：无法执行 {act} {obj}");
        Err(loco_rs::Error::CustomError(
            StatusCode::FORBIDDEN,
            ErrorDetail::new("authz.insufficient_permissions", &desc),
        ))
    }
}
