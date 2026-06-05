use knota_fold::app::App;
use knota_fold::views::auth::LoginResponse;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

#[tokio::test]
#[serial]
async fn unauthenticated_list_returns_error() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/users").await;
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
async fn can_list_users() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/users?page=1&pageSize=10")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "List users should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let total = body["totalItems"].as_i64().unwrap();
        assert!(
            total >= 3,
            "Expected at least 3 seeded users (super admin + user1 + user2), got {total}"
        );

        let items = body["items"].as_array().unwrap();
        assert!(!items.is_empty(), "Items array should not be empty");

        let first = &items[0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(
            first.get("tenantCode").is_some(),
            "Missing 'tenantCode' key"
        );
        assert!(first.get("email").is_some(), "Missing 'email' key");
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("status").is_some(), "Missing 'status' key");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_user() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let payload = serde_json::json!({
            "email": "newuser@test.com",
            "password": "password1234",
            "name": "New User",
        });

        let response = request
            .post("/api/users")
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "Create user should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["email"], "newuser@test.com");
        assert_eq!(body["name"], "New User");
        assert_eq!(body["tenantCode"], "DEFAULT");
        assert_eq!(body["status"], "active");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_update_user() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a user first
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "email": "update-me@test.com",
            "password": "password1234",
            "name": "Before Update",
        });
        let create_response = request
            .post("/api/users")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let user_id = created["id"].as_str().expect("User should have id");

        // Update the user
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let update_payload = serde_json::json!({
            "name": "After Update",
        });
        let update_response = request
            .put(&format!("/api/users/{user_id}"))
            .json(&update_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            update_response.status_code(),
            200,
            "Update user should succeed"
        );

        let body: serde_json::Value =
            serde_json::from_str(&update_response.text()).unwrap();
        assert_eq!(body["name"], "After Update");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_toggle_user_status() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a user to toggle
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "email": "toggle-me@test.com",
            "password": "password1234",
            "name": "To Toggle",
        });
        let create_response = request
            .post("/api/users")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let user_id = created["id"].as_str().expect("User should have id");
        assert_eq!(created["status"], "active", "New user should be active");

        // Disable the user
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let disable_payload = serde_json::json!({ "status": "disabled" });
        let disable_response = request
            .put(&format!("/api/users/{user_id}/status"))
            .json(&disable_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            disable_response.status_code(),
            200,
            "Disable user should succeed"
        );

        let disabled: serde_json::Value =
            serde_json::from_str(&disable_response.text()).unwrap();
        assert_eq!(
            disabled["status"], "disabled",
            "User should now be disabled"
        );

        // Re-enable the user
        let (k3, v3) = prepare_data::auth_header(&admin.token);
        let enable_payload = serde_json::json!({ "status": "active" });
        let enable_response = request
            .put(&format!("/api/users/{user_id}/status"))
            .json(&enable_payload)
            .add_header(k3, v3)
            .await;
        assert_eq!(
            enable_response.status_code(),
            200,
            "Enable user should succeed"
        );

        let enabled: serde_json::Value =
            serde_json::from_str(&enable_response.text()).unwrap();
        assert_eq!(enabled["status"], "active", "User should be active again");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn disabled_user_cannot_login() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a user
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "email": "disabled-login@test.com",
            "password": "password1234",
            "name": "Disabled Login User",
        });
        let create_response = request
            .post("/api/users")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let user_id = created["id"].as_str().expect("User should have id");

        // Verify the user can login
        let login_response = request
            .post("/api/auth/login")
            .json(&serde_json::json!({
                "email": "disabled-login@test.com",
                "password": "password1234",
            }))
            .await;
        assert_eq!(
            login_response.status_code(),
            200,
            "Active user should login"
        );

        // Disable the user
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let disable_payload = serde_json::json!({ "status": "disabled" });
        let disable_response = request
            .put(&format!("/api/users/{user_id}/status"))
            .json(&disable_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            disable_response.status_code(),
            200,
            "Disable should succeed"
        );

        // Attempt to login as disabled user — should fail
        let login_response2 = request
            .post("/api/auth/login")
            .json(&serde_json::json!({
                "email": "disabled-login@test.com",
                "password": "password1234",
            }))
            .await;
        assert_eq!(
            login_response2.status_code(),
            401,
            "Disabled user should not be able to login"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_reset_password() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a user
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "email": "reset-pwd@test.com",
            "password": "old-password",
            "name": "Reset Pwd User",
        });
        let create_response = request
            .post("/api/users")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let user_id = created["id"].as_str().expect("User should have id");

        // Reset password
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let reset_payload = serde_json::json!({
            "password": "new-password",
        });
        let reset_response = request
            .put(&format!("/api/users/{user_id}/reset-password"))
            .json(&reset_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            reset_response.status_code(),
            200,
            "Reset password should succeed"
        );

        // Login with new password
        let login_response = request
            .post("/api/auth/login")
            .json(&serde_json::json!({
                "email": "reset-pwd@test.com",
                "password": "new-password",
            }))
            .await;
        assert_eq!(
            login_response.status_code(),
            200,
            "Login with new password should succeed"
        );

        let lr: LoginResponse = serde_json::from_str(&login_response.text()).unwrap();
        assert!(!lr.token.is_empty(), "Login should return a valid token");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn super_admin_cannot_disable_self() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let user_id = admin.user.id.to_string();
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .put(&format!("/api/users/{user_id}/status"))
            .json(&serde_json::json!({ "status": "disabled" }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            400,
            "Super admin should not be able to disable themselves: {}",
            response.text()
        );
        let body = response.text();
        assert!(
            body.contains("管理员不能禁用自己的帐户"),
            "Error message mismatch: {body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/users/super-admin")
            .json(&serde_json::json!({
                "email": "new-sa@test.com",
                "password": "sa-password",
                "name": "New Super Admin",
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create super admin should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["email"], "new-sa@test.com");
        assert_eq!(body["tenantCode"], "DEFAULT");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn non_super_admin_cannot_create_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let ta = prepare_data::create_tenant_and_login_admin(
            &request,
            &admin.token,
            "Test Tenant",
            "TEST_T",
            "tadmin@test.com",
            "tadmin-pwd",
            "Tenant Admin",
        )
        .await;
        let (k, v) = prepare_data::auth_header(&ta.token);

        let response = request
            .post("/api/users/super-admin")
            .json(&serde_json::json!({
                "email": "blocked-sa@test.com",
                "password": "sa-password",
                "name": "Blocked Super Admin",
            }))
            .add_header(k, v)
            .await;

        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403, got {status}: {}",
            response.text()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn new_super_admin_can_login_and_access_admin_endpoints() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k1, v1) = prepare_data::auth_header(&admin.token);

        let create_response = request
            .post("/api/users/super-admin")
            .json(&serde_json::json!({
                "email": "fresh-sa@test.com",
                "password": "sa-password",
                "name": "Fresh Super Admin",
            }))
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create super admin should succeed"
        );

        let login_response = request
            .post("/api/auth/login")
            .json(&serde_json::json!({
                "email": "fresh-sa@test.com",
                "password": "sa-password",
            }))
            .await;
        assert_eq!(
            login_response.status_code(),
            200,
            "New super admin should be able to login"
        );

        let lr: LoginResponse = serde_json::from_str(&login_response.text()).unwrap();
        let (k2, v2) = prepare_data::auth_header(&lr.token);

        let list_response = request
            .get("/api/users?page=1&pageSize=10")
            .add_header(k2, v2)
            .await;
        assert_eq!(
            list_response.status_code(),
            200,
            "New super admin should access admin endpoints"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_user_roles() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_response = request
            .post("/api/users")
            .json(&serde_json::json!({
                "email": "roles-user@test.com",
                "password": "password1234",
                "name": "Roles User",
            }))
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let user_id = created["id"].as_str().expect("User should have id");

        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let roles_response = request
            .get("/api/roles?page=1&pageSize=10")
            .add_header(k2, v2)
            .await;
        assert_eq!(
            roles_response.status_code(),
            200,
            "List roles should succeed"
        );

        let roles_body: serde_json::Value =
            serde_json::from_str(&roles_response.text()).unwrap();
        let role_id = roles_body["items"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|role| role["id"].as_str())
            .expect("Roles list should contain at least one role id");

        let (k3, v3) = prepare_data::auth_header(&admin.token);
        let sync_response = request
            .put(&format!("/api/users/{user_id}/roles"))
            .json(&serde_json::json!({
                "roleIds": [role_id]
            }))
            .add_header(k3, v3)
            .await;
        assert_eq!(
            sync_response.status_code(),
            200,
            "Sync user roles should succeed"
        );

        let (k4, v4) = prepare_data::auth_header(&admin.token);
        let get_response = request
            .get(&format!("/api/users/{user_id}/roles"))
            .add_header(k4, v4)
            .await;
        assert_eq!(
            get_response.status_code(),
            200,
            "Get user roles should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&get_response.text()).unwrap();
        let role_ids = body["roleIds"]
            .as_array()
            .expect("roleIds should be an array");
        let ids: Vec<&str> = role_ids.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            ids.contains(&role_id),
            "Synced role ID {role_id} should be present in {ids:?}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_user_roles_empty() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_response = request
            .post("/api/users")
            .json(&serde_json::json!({
                "email": "roles-empty@test.com",
                "password": "password1234",
                "name": "Roles Empty User",
            }))
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create user should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let user_id = created["id"].as_str().expect("User should have id");

        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let get_response = request
            .get(&format!("/api/users/{user_id}/roles"))
            .add_header(k2, v2)
            .await;
        assert_eq!(
            get_response.status_code(),
            200,
            "Get user roles should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&get_response.text()).unwrap();
        let role_ids = body["roleIds"]
            .as_array()
            .expect("roleIds should be an array");
        assert!(
            role_ids.is_empty(),
            "roleIds should be empty, got {role_ids:?}"
        );
    })
    .await;
}
