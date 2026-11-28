use loco_rs::app::AppContext;

use crate::services::sys_config_service;

pub const KEY_REGISTRATION_ENABLED: &str = "auth.registration.enabled";

const DEFAULT_REGISTRATION_ENABLED: bool = false;

async fn load_bool(ctx: &AppContext, key: &str, default: bool) -> bool {
    match sys_config_service::get_resolved_detail(ctx, key, None).await {
        Ok(Some(detail)) => match detail.resolved_value.as_str() {
            "true" => true,
            "false" => false,
            _ => default,
        },
        _ => default,
    }
}

pub async fn registration_enabled(ctx: &AppContext) -> bool {
    load_bool(ctx, KEY_REGISTRATION_ENABLED, DEFAULT_REGISTRATION_ENABLED).await
}
