use axum::http::StatusCode;
use casbin::prelude::*;
use loco_rs::controller::ErrorDetail;
use sea_orm::DatabaseConnection;
use sea_orm_adapter::SeaOrmAdapter;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type SharedEnforcer = Arc<RwLock<Enforcer>>;

pub async fn init_enforcer(db: &DatabaseConnection) -> loco_rs::Result<SharedEnforcer> {
    let model = DefaultModel::from_file("config/casbin/model.conf")
        .await
        .map_err(|e| {
            let desc = format!("Failed to load casbin model: {e}");
            loco_rs::Error::CustomError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorDetail::new("casbin.model_load_failed", &desc),
            )
        })?;
    let adapter = SeaOrmAdapter::new(db.clone()).await.map_err(|e| {
        let desc = format!("Failed to create casbin adapter: {e}");
        loco_rs::Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.adapter_failed", &desc),
        )
    })?;
    let mut e = Enforcer::new(model, adapter).await.map_err(|e| {
        let desc = format!("Failed to create enforcer: {e}");
        loco_rs::Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.enforcer_failed", &desc),
        )
    })?;
    e.enable_auto_save(true);
    e.load_policy().await.map_err(|e| {
        let desc = format!("Failed to load casbin policy: {e}");
        loco_rs::Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new("casbin.policy_load_failed", &desc),
        )
    })?;
    Ok(Arc::new(RwLock::new(e)))
}

pub async fn sync_user_roles(
    enforcer: &SharedEnforcer,
    user_id: &str,
    tenant_id: &str,
    role_codes: &[String],
) -> casbin::Result<()> {
    let mut e = enforcer.write().await;
    e.remove_filtered_grouping_policy(
        0,
        vec![user_id.to_string(), String::new(), tenant_id.to_string()],
    )
    .await?;
    for role_code in role_codes {
        e.add_grouping_policy(vec![
            user_id.to_string(),
            role_code.clone(),
            tenant_id.to_string(),
        ])
        .await?;
    }
    Ok(())
}

pub async fn sync_role_permissions(
    enforcer: &SharedEnforcer,
    role_code: &str,
    tenant_id: &str,
    permissions: &[(String, String)],
) -> casbin::Result<()> {
    let mut e = enforcer.write().await;
    e.remove_filtered_policy(0, vec![role_code.to_string(), tenant_id.to_string()])
        .await?;
    for (obj, act) in permissions {
        e.add_policy(vec![
            role_code.to_string(),
            tenant_id.to_string(),
            obj.clone(),
            act.clone(),
        ])
        .await?;
    }
    Ok(())
}

pub async fn sync_api_key_role(
    enforcer: &SharedEnforcer,
    api_key_id: &str,
    role_code: &str,
    tenant_id: &str,
) -> casbin::Result<()> {
    let subject = format!("apikey:{api_key_id}");
    let mut e = enforcer.write().await;
    e.remove_filtered_grouping_policy(
        0,
        vec![subject.clone(), String::new(), tenant_id.to_string()],
    )
    .await?;
    e.add_grouping_policy(vec![subject, role_code.to_string(), tenant_id.to_string()])
        .await?;
    Ok(())
}

pub async fn remove_api_key_role(
    enforcer: &SharedEnforcer,
    api_key_id: &str,
    tenant_id: &str,
) -> casbin::Result<()> {
    let mut e = enforcer.write().await;
    e.remove_filtered_grouping_policy(
        0,
        vec![
            format!("apikey:{api_key_id}"),
            String::new(),
            tenant_id.to_string(),
        ],
    )
    .await?;
    Ok(())
}

pub async fn remove_role_policies(
    enforcer: &SharedEnforcer,
    role_code: &str,
    tenant_id: &str,
) -> casbin::Result<()> {
    let mut e = enforcer.write().await;
    e.remove_filtered_policy(0, vec![role_code.to_string(), tenant_id.to_string()])
        .await?;
    e.remove_filtered_grouping_policy(
        1,
        vec![role_code.to_string(), tenant_id.to_string()],
    )
    .await?;
    Ok(())
}

pub async fn remove_permission_policies(
    enforcer: &SharedEnforcer,
    obj: &str,
    act: &str,
) -> casbin::Result<()> {
    let mut e = enforcer.write().await;
    // Remove all policies matching (*, *, obj, act) across all tenants
    e.remove_filtered_policy(2, vec![obj.to_string(), act.to_string()])
        .await?;
    Ok(())
}

pub async fn enforce_check(
    enforcer: &SharedEnforcer,
    user_id: &str,
    tenant_id: &str,
    obj: &str,
    act: &str,
) -> casbin::Result<bool> {
    let e = enforcer.read().await;
    e.enforce((user_id, tenant_id, obj, act))
}
