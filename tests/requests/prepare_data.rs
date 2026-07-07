#![allow(dead_code)]

use axum::http::{HeaderName, HeaderValue};

use knota_fold::{
    app::App,
    models::{_entities::sys_configs, users},
    services::auth_policy,
    views::auth::LoginResponse,
};
use loco_rs::TestServer;
use loco_rs::{app::AppContext, testing::prelude::seed};
use sea_orm::{sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter};

const USER_EMAIL: &str = "test@loco.com";
const USER_PASSWORD: &str = "1234";

pub struct LoggedInUser {
    pub user: users::Model,
    pub token: String,
}

async fn set_registration_enabled(ctx: &AppContext, enabled: bool) {
    sys_configs::Entity::update_many()
        .col_expr(sys_configs::Column::Value, Expr::value(enabled.to_string()))
        .filter(sys_configs::Column::Key.eq(auth_policy::KEY_REGISTRATION_ENABLED))
        .filter(sys_configs::Column::TenantId.is_null())
        .exec(&ctx.db)
        .await
        .expect("failed to update registration config");

    let _ = ctx
        .cache
        .remove(&format!(
            "cfg:resolved:global:{}",
            auth_policy::KEY_REGISTRATION_ENABLED
        ))
        .await;
    let _ = ctx.cache.remove("cfg:all:global").await;
}

pub async fn init_user_login(request: &TestServer, ctx: &AppContext) -> LoggedInUser {
    seed::<App>(ctx).await.unwrap();
    set_registration_enabled(ctx, true).await;

    let register_payload = serde_json::json!({
        "name": "loco",
        "email": USER_EMAIL,
        "password": USER_PASSWORD
    });

    //Creating a new user
    request
        .post("/api/auth/register")
        .json(&register_payload)
        .await;
    let user = users::Model::find_by_email(&ctx.db, USER_EMAIL)
        .await
        .unwrap();

    let verify_token = user.email_verification_token.clone().unwrap();
    request
        .get(&format!("/api/auth/verify/{verify_token}"))
        .await;

    let response = request
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "email": USER_EMAIL,
            "password": USER_PASSWORD
        }))
        .await;

    let login_response: LoginResponse = serde_json::from_str(&response.text()).unwrap();

    LoggedInUser {
        user: users::Model::find_by_email(&ctx.db, USER_EMAIL)
            .await
            .unwrap(),
        token: login_response.token,
    }
}

/// Log in an already-seeded user (no registration, no email verification).
pub async fn login_seeded_user(
    request: &TestServer,
    ctx: &AppContext,
    email: &str,
    password: &str,
) -> LoggedInUser {
    seed::<App>(ctx).await.unwrap();

    let response = request
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "email": email,
            "password": password
        }))
        .await;

    assert_eq!(
        response.status_code(),
        200,
        "Seeded user login failed for {email}"
    );

    let login_response: LoginResponse = serde_json::from_str(&response.text()).unwrap();

    LoggedInUser {
        user: users::Model::find_by_email(&ctx.db, email).await.unwrap(),
        token: login_response.token,
    }
}

const SUPER_ADMIN_EMAIL: &str = "super.admin@knota.com";
const SUPER_ADMIN_PASSWORD: &str = "super.admin.pwd2048";

/// Convenience: seed + login as the platform super admin.
pub async fn login_super_admin(request: &TestServer, ctx: &AppContext) -> LoggedInUser {
    login_seeded_user(request, ctx, SUPER_ADMIN_EMAIL, SUPER_ADMIN_PASSWORD).await
}

pub fn auth_header(token: &str) -> (HeaderName, HeaderValue) {
    let auth_header_value = HeaderValue::from_str(&format!("Bearer {}", &token)).unwrap();

    (HeaderName::from_static("authorization"), auth_header_value)
}

/// Tenant admin info returned by [`create_tenant_and_login_admin`].
pub struct TenantAdmin {
    pub tenant_id: String,
    pub token: String,
}

/// Create a new tenant (with full init) via super admin, then create a tenant
/// admin, then login as that admin. Returns [`TenantAdmin`] with the tenant id
/// and the admin's JWT token.
///
/// Callers must have already called [`login_super_admin`] (which seeds fixtures)
/// and pass the super admin token.
pub async fn create_tenant_and_login_admin(
    request: &TestServer,
    super_token: &str,
    tenant_name: &str,
    tenant_code: &str,
    admin_email: &str,
    admin_password: &str,
    admin_name: &str,
) -> TenantAdmin {
    // 1. Create tenant
    let (k, v) = auth_header(super_token);
    let create_response = request
        .post("/api/tenants")
        .json(&serde_json::json!({
            "name": tenant_name,
            "code": tenant_code,
        }))
        .add_header(k, v)
        .await;
    assert_eq!(
        create_response.status_code(),
        200,
        "Create tenant '{tenant_code}' failed: {}",
        create_response.text()
    );
    let tenant_body: serde_json::Value =
        serde_json::from_str(&create_response.text()).unwrap();
    let tenant_id = tenant_body["id"]
        .as_str()
        .expect("tenant response should have 'id'")
        .to_string();

    // 2. Create admin for the new tenant
    let (k, v) = auth_header(super_token);
    let admin_response = request
        .post(&format!("/api/sys/tenants/{tenant_code}/admins"))
        .json(&serde_json::json!({
            "email": admin_email,
            "password": admin_password,
            "name": admin_name,
        }))
        .add_header(k, v)
        .await;
    assert_eq!(
        admin_response.status_code(),
        200,
        "Create admin for tenant '{tenant_code}' failed: {}",
        admin_response.text()
    );

    // 3. Login as the new admin
    let login_response = request
        .post("/api/auth/login")
        .json(&serde_json::json!({
            "email": admin_email,
            "password": admin_password,
        }))
        .await;
    assert_eq!(
        login_response.status_code(),
        200,
        "Login as tenant admin '{admin_email}' failed: {}",
        login_response.text()
    );

    let lr: LoginResponse = serde_json::from_str(&login_response.text()).unwrap();

    TenantAdmin {
        tenant_id,
        token: lr.token,
    }
}
