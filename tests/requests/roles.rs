use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

const SUPER_ADMIN_ROLE_ID: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaa001";
const TENANT_ADMIN_ROLE_ID: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaa004";
const PERMISSION_ID: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb001";
const SYS_MENU_ID: &str = "cccccccc-cccc-cccc-cccc-ccccccccc001";

#[tokio::test]
#[serial]
async fn unauthenticated_list_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/roles").await;
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
async fn can_list_roles() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/roles?page=1&pageSize=10")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "List roles should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(
            body["totalItems"], 2,
            "Expected 2 seeded roles (SUPER_ADMIN, TENANT_ADMIN)"
        );

        let first = &body["items"][0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(
            first.get("tenantCode").is_some(),
            "Missing 'tenantCode' key"
        );
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("code").is_some(), "Missing 'code' key");
        assert!(first.get("parentId").is_some(), "Missing 'parentId' key");
        assert!(first.get("isSystem").is_some(), "Missing 'isSystem' key");
        assert!(
            first.get("description").is_some(),
            "Missing 'description' key"
        );
        assert!(first.get("version").is_some(), "Missing 'version' key");
        assert!(first.get("status").is_some(), "Missing 'status' key");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_role() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let payload = serde_json::json!({
            "name": "Auditor",
            "code": "AUDITOR",
            "description": "Audit role"
        });

        let response = request
            .post("/api/roles")
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "Create role should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["name"], "Auditor");
        assert_eq!(body["code"], "AUDITOR");
        assert_eq!(body["tenantCode"], "DEFAULT");
        assert_eq!(body["version"], 1);
        assert_eq!(body["status"], "active");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_update_role() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (create_key, create_value) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Update Target",
            "code": "UPDATE_TARGET",
            "description": "Role created for update test"
        });
        let create_response = request
            .post("/api/roles")
            .json(&create_payload)
            .add_header(create_key, create_value)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create role for update should succeed: {}",
            create_response.text()
        );
        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let role_id = created["id"].as_str().expect("Created role should have id");

        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let payload = serde_json::json!({
            "name": "Update Target Renamed",
            "version": 1
        });

        let response = request
            .put(&format!("/api/roles/{role_id}"))
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "Update role should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["name"], "Update Target Renamed");
        assert_eq!(body["version"], 2);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_toggle_role_status() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a role to toggle
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Toggle Role",
            "code": "TOGGLE_ROLE",
            "description": "To be toggled"
        });
        let create_response = request
            .post("/api/roles")
            .json(&create_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create role should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let role_id = created["id"]
            .as_str()
            .expect("Created role should have an id");
        assert_eq!(created["status"], "active", "New role should be active");

        // Disable the role
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let disable_payload = serde_json::json!({ "status": "disabled" });
        let disable_response = request
            .put(&format!("/api/roles/{role_id}/status"))
            .json(&disable_payload)
            .add_header(k2, v2)
            .await;
        assert_eq!(
            disable_response.status_code(),
            200,
            "Disable role should succeed"
        );

        let disabled: serde_json::Value =
            serde_json::from_str(&disable_response.text()).unwrap();
        assert_eq!(
            disabled["status"], "disabled",
            "Role should now be disabled"
        );

        // Re-enable the role
        let (k3, v3) = prepare_data::auth_header(&admin.token);
        let enable_payload = serde_json::json!({ "status": "active" });
        let enable_response = request
            .put(&format!("/api/roles/{role_id}/status"))
            .json(&enable_payload)
            .add_header(k3, v3)
            .await;
        assert_eq!(
            enable_response.status_code(),
            200,
            "Enable role should succeed"
        );

        let enabled: serde_json::Value =
            serde_json::from_str(&enable_response.text()).unwrap();
        assert_eq!(enabled["status"], "active", "Role should be active again");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_role_permissions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get(&format!("/api/roles/{SUPER_ADMIN_ROLE_ID}/permissions"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Get role permissions should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let permission_ids = body["permissionIds"]
            .as_array()
            .expect("permissionIds should be an array");
        assert!(
            permission_ids.len() >= 30,
            "SUPER_ADMIN should have all permissions (>= 30), got {}",
            permission_ids.len()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_sync_role_permissions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (create_key, create_value) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Permission Sync Target",
            "code": "PERMISSION_SYNC_TARGET",
            "description": "Role created for permission sync test"
        });
        let create_response = request
            .post("/api/roles")
            .json(&create_payload)
            .add_header(create_key, create_value)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create role for permission sync should succeed: {}",
            create_response.text()
        );
        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let role_id = created["id"].as_str().expect("Created role should have id");

        // Sync permissions
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let sync_payload = serde_json::json!({
            "permissionIds": [PERMISSION_ID]
        });
        let sync_response = request
            .put(&format!("/api/roles/{role_id}/permissions"))
            .json(&sync_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            sync_response.status_code(),
            200,
            "Sync role permissions should succeed"
        );

        // Verify persisted
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let get_response = request
            .get(&format!("/api/roles/{role_id}/permissions"))
            .add_header(k2, v2)
            .await;
        assert_eq!(
            get_response.status_code(),
            200,
            "Get role permissions should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&get_response.text()).unwrap();
        let permission_ids = body["permissionIds"]
            .as_array()
            .expect("permissionIds should be an array");
        let ids: Vec<&str> = permission_ids.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            ids.contains(&PERMISSION_ID),
            "Synced permission ID {PERMISSION_ID} should be present in {ids:?}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_role_menus() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get(&format!("/api/roles/{TENANT_ADMIN_ROLE_ID}/menus"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "Get role menus should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(
            body.get("sysMenuIds").is_some(),
            "Response should contain sysMenuIds"
        );
        assert!(
            body["sysMenuIds"].is_array(),
            "sysMenuIds should be an array"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_sync_role_menus() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (create_key, create_value) = prepare_data::auth_header(&admin.token);
        let create_payload = serde_json::json!({
            "name": "Menu Sync Target",
            "code": "MENU_SYNC_TARGET",
            "description": "Role created for menu sync test"
        });
        let create_response = request
            .post("/api/roles")
            .json(&create_payload)
            .add_header(create_key, create_value)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create role for menu sync should succeed: {}",
            create_response.text()
        );
        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let role_id = created["id"].as_str().expect("Created role should have id");

        // Sync menus
        let (k1, v1) = prepare_data::auth_header(&admin.token);
        let sync_payload = serde_json::json!({
            "sysMenuIds": [SYS_MENU_ID]
        });
        let sync_response = request
            .put(&format!("/api/roles/{role_id}/menus"))
            .json(&sync_payload)
            .add_header(k1, v1)
            .await;
        assert_eq!(
            sync_response.status_code(),
            200,
            "Sync role menus should succeed"
        );

        // Verify persisted
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let get_response = request
            .get(&format!("/api/roles/{role_id}/menus"))
            .add_header(k2, v2)
            .await;
        assert_eq!(
            get_response.status_code(),
            200,
            "Get role menus should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&get_response.text()).unwrap();
        let menu_ids = body["sysMenuIds"]
            .as_array()
            .expect("sysMenuIds should be an array");
        let ids: Vec<&str> = menu_ids.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            ids.contains(&SYS_MENU_ID),
            "Synced menu ID {SYS_MENU_ID} should be present in {ids:?}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_assignable_permissions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get(&format!(
                "/api/roles/{TENANT_ADMIN_ROLE_ID}/assignable-permissions"
            ))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Get assignable permissions should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();

        let permissions = body["permissions"]
            .as_array()
            .expect("permissions should be an array");
        assert!(
            !permissions.is_empty(),
            "Should contain at least one permission"
        );

        let first = &permissions[0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("code").is_some(), "Missing 'code' key");
        assert!(first.get("obj").is_some(), "Missing 'obj' key");
        assert!(first.get("act").is_some(), "Missing 'act' key");
        assert!(first.get("type").is_some(), "Missing 'type' key");
        assert!(first.get("tag").is_some(), "Missing 'tag' key");
        assert!(
            first.get("description").is_some(),
            "Missing 'description' key"
        );

        let assigned = body["assignedPermissionIds"]
            .as_array()
            .expect("assignedPermissionIds should be an array");
        // Verify it's a valid array (already guaranteed by as_array unwrap above).
        // ADMIN_ROLE_ID starts with no permissions in a fresh test DB.
        let _ = assigned;
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_list_roles_with_tenant_code() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/roles?page=1&pageSize=10&tenantCode=DEFAULT")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "List roles with tenantCode should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let total = body["totalItems"].as_i64().unwrap();
        assert!(total >= 1, "Expected at least 1 role for DEFAULT tenant");

        let items = body["items"].as_array().unwrap();
        for item in items {
            assert_eq!(
                item["tenantCode"].as_str().unwrap(),
                "DEFAULT",
                "All returned roles should belong to DEFAULT tenant"
            );
        }
    })
    .await;
}
