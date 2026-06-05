use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

#[tokio::test]
#[serial]
async fn unauthenticated_list_returns_error() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/tenants").await;
        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403, got {status}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_list_tenants() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/tenants?page=1&pageSize=10")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "List tenants should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let total = body["totalItems"].as_i64().unwrap();
        assert!(
            total >= 1,
            "Expected at least 1 tenant (DEFAULT), got {total}"
        );

        let items = body["items"].as_array().unwrap();
        assert!(!items.is_empty(), "Items array should not be empty");

        let first = &items[0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("code").is_some(), "Missing 'code' key");
        assert!(first.get("status").is_some(), "Missing 'status' key");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let payload = serde_json::json!({
            "name": "Test Create Tenant",
            "code": "TEST_CREATE",
        });

        let response = request
            .post("/api/tenants")
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "Create tenant should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["code"], "TEST_CREATE");
        assert_eq!(body["name"], "Test Create Tenant");
        assert!(body.get("id").is_some(), "Response should have 'id'");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_update_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a tenant first
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Before Update",
            "code": "UPDATE_TEST",
        });
        let create_response = request
            .post("/api/tenants")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create tenant should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let tenant_id = created["id"].as_str().expect("Tenant should have id");

        // Update the tenant
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let update_payload = serde_json::json!({
            "name": "After Update",
        });
        let update_response = request
            .put(&format!("/api/tenants/{tenant_id}"))
            .json(&update_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            update_response.status_code(),
            200,
            "Update tenant should succeed"
        );

        let body: serde_json::Value =
            serde_json::from_str(&update_response.text()).unwrap();
        assert_eq!(body["name"], "After Update");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_toggle_tenant_status() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a tenant
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Toggle Test",
            "code": "TOGGLE_TEST",
        });
        let create_response = request
            .post("/api/tenants")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create tenant should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let tenant_id = created["id"].as_str().expect("Tenant should have id");

        // Disable the tenant via update
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let update_payload = serde_json::json!({ "status": "inactive" });
        let update_response = request
            .put(&format!("/api/tenants/{tenant_id}"))
            .json(&update_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            update_response.status_code(),
            200,
            "Toggle tenant status should succeed"
        );

        let body: serde_json::Value =
            serde_json::from_str(&update_response.text()).unwrap();
        assert_eq!(body["status"], "inactive");

        // Re-enable the tenant
        let (k3, v3) = prepare_data::auth_header(&admin.token);
        let enable_payload = serde_json::json!({ "status": "active" });
        let enable_response = request
            .put(&format!("/api/tenants/{tenant_id}"))
            .json(&enable_payload)
            .add_header(k3, v3)
            .await;
        assert_eq!(
            enable_response.status_code(),
            200,
            "Re-enable tenant should succeed"
        );

        let body2: serde_json::Value =
            serde_json::from_str(&enable_response.text()).unwrap();
        assert_eq!(body2["status"], "active");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_tenant_roles() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/sys/tenants/DEFAULT/roles")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Get tenant roles should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let roles = body
            .as_array()
            .expect("Response should be an array of roles");
        assert_eq!(
            roles.len(),
            2,
            "DEFAULT tenant should have 2 roles (SUPER_ADMIN, TENANT_ADMIN), got {}",
            roles.len()
        );

        let first = &roles[0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(
            first.get("tenantCode").is_some(),
            "Missing 'tenantCode' key"
        );
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("code").is_some(), "Missing 'code' key");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_tenant_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a tenant first
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Admin Test Tenant",
            "code": "ADMIN_TEST",
        });
        let create_response = request
            .post("/api/tenants")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create tenant should succeed"
        );

        // Create an admin for the tenant
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let admin_payload = serde_json::json!({
            "email": "tenant-admin@test.com",
            "password": "admin1234",
            "name": "Tenant Admin",
        });
        let admin_response = request
            .post("/api/sys/tenants/ADMIN_TEST/admins")
            .json(&admin_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            admin_response.status_code(),
            200,
            "Create tenant admin should succeed"
        );

        let body: serde_json::Value =
            serde_json::from_str(&admin_response.text()).unwrap();
        assert_eq!(body["tenantCode"], "ADMIN_TEST");
        assert_eq!(body["email"], "tenant-admin@test.com");
        assert_eq!(body["name"], "Tenant Admin");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn new_tenant_admin_can_login() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &admin.token,
            "Login Test Tenant",
            "LOGIN_TEST",
            "login-admin@test.com",
            "admin1234",
            "Login Admin",
        )
        .await;

        // Token should be non-empty
        assert!(
            !ta.token.is_empty(),
            "Tenant admin should receive a valid JWT token"
        );
        assert!(!ta.tenant_id.is_empty(), "Tenant id should be present");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn new_tenant_has_seeded_roles() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a new tenant
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Seeded Roles Tenant",
            "code": "SEED_ROLES",
        });
        let create_response = request
            .post("/api/tenants")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create tenant should succeed"
        );

        // Get roles for the new tenant
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let roles_response = request
            .get("/api/sys/tenants/SEED_ROLES/roles")
            .add_header(k2, v2)
            .await;
        assert_eq!(
            roles_response.status_code(),
            200,
            "Get tenant roles should succeed"
        );

        let body: serde_json::Value =
            serde_json::from_str(&roles_response.text()).unwrap();
        let roles = body
            .as_array()
            .expect("Response should be an array of roles");
        assert_eq!(
            roles.len(),
            2,
            "New tenant should have 2 seeded roles (TENANT_ADMIN, MEMBER), got {}",
            roles.len()
        );

        let codes: Vec<&str> =
            roles.iter().map(|r| r["code"].as_str().unwrap()).collect();
        assert!(
            codes.contains(&"TENANT_ADMIN"),
            "Should have TENANT_ADMIN role, got {codes:?}"
        );
        assert!(
            codes.contains(&"MEMBER"),
            "Should have MEMBER role, got {codes:?}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn cannot_disable_default_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Get default tenant ID from the list
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let list_response = request
            .get("/api/tenants?page=1&pageSize=10")
            .add_header(k1, v1)
            .await;
        assert_eq!(list_response.status_code(), 200);
        let list_body: serde_json::Value =
            serde_json::from_str(&list_response.text()).unwrap();
        let items = list_body["items"].as_array().unwrap();
        let default_tenant = items
            .iter()
            .find(|t| t["code"] == "DEFAULT")
            .expect("Default tenant should exist");
        let default_tenant_id = default_tenant["id"].as_str().unwrap();

        // Try to disable the default tenant
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/tenants/{default_tenant_id}"))
            .json(&serde_json::json!({ "status": "inactive" }))
            .add_header(k2, v2)
            .await;

        assert_eq!(
            response.status_code(),
            400,
            "Default tenant should not be disableable: {}",
            response.text()
        );
        let body = response.text();
        assert!(
            body.contains("默认租户不允许被禁用"),
            "Error message mismatch: {body}"
        );
    })
    .await;
}
