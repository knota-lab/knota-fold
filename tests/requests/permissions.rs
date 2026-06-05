use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

const PERMISSION_ROLE_READ_ID: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb001";
const PERMISSION_MENU_READ_ID: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb003";

#[tokio::test]
#[serial]
async fn unauthenticated_list_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/permissions").await;
        assert!(
            response.status_code() == 401 || response.status_code() == 403,
            "Expected 401 or 403 for unauthenticated request, got {}",
            response.status_code()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_list_permissions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/permissions?page=1&pageSize=10")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "List permissions should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let total = body["totalItems"].as_i64().unwrap();
        assert!(total >= 30, "Expected at least 30 permissions, got {total}");

        let items = body["items"].as_array().unwrap();
        assert!(!items.is_empty(), "Items array should not be empty");

        let first = &items[0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("code").is_some(), "Missing 'code' key");
        assert!(first.get("obj").is_some(), "Missing 'obj' key");
        assert!(first.get("act").is_some(), "Missing 'act' key");
        assert!(first.get("type").is_some(), "Missing 'type' key");
        assert!(first.get("isSystem").is_some(), "Missing 'isSystem' key");
        assert!(first.get("version").is_some(), "Missing 'version' key");
        // Should NOT have tenantCode (removed after refactoring)
        assert!(
            first.get("tenantCode").is_none(),
            "tenantCode should not be present"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_list_permissions_paginated() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/permissions?page=1&pageSize=5")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "Paginated list should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 5, "Expected exactly 5 items per page");

        let total_items = body["totalItems"].as_i64().unwrap();
        assert!(
            total_items >= 30,
            "Expected at least 30 total items, got {total_items}"
        );

        let total_pages = body["totalPages"].as_i64().unwrap();
        assert!(
            total_pages >= 6,
            "Expected at least 6 total pages, got {total_pages}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_update_permission() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let payload = serde_json::json!({
            "name": "Read Roles HTTP",
            "version": 1
        });

        let response = request
            .put(&format!("/api/permissions/{PERMISSION_ROLE_READ_ID}"))
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Update permission should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(
            body["name"].as_str().unwrap(),
            "Read Roles HTTP",
            "Name should be updated"
        );
        assert_eq!(
            body["version"].as_i64().unwrap(),
            2,
            "Version should be incremented to 2"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_delete_permission() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .delete(&format!("/api/permissions/{PERMISSION_MENU_READ_ID}"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Delete permission should succeed"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_permissions_with_metadata() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/permissions/with-metadata")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Get permissions with metadata should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let items = body["permissions"]
            .as_array()
            .expect("permissions should be an array");
        assert!(!items.is_empty(), "Should contain at least one permission");
        assert!(
            body["unmatchedRoutes"].is_array(),
            "unmatchedRoutes should be an array"
        );

        let first = &items[0];
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
    })
    .await;
}

#[tokio::test]
#[serial]
async fn non_super_admin_with_metadata_returns_error() {
    request::<App, _, _>(|request, ctx| async move {
        let user = prepare_data::init_user_login(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&user.token);

        let response = request
            .get("/api/permissions/with-metadata")
            .add_header(auth_key, auth_value)
            .await;

        assert_ne!(
            response.status_code(),
            200,
            "Non-super-admin should not be able to access with-metadata"
        );
        assert!(
            response.status_code() == 401 || response.status_code() == 403,
            "Expected 401 or 403 for non-super-admin, got {}",
            response.status_code()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_sync_permissions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let payload = serde_json::json!({
            "items": [{"path": "/api/test-sync/", "method": "GET"}]
        });

        let response = request
            .post("/api/permissions/sync")
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Sync permissions should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let items = body.as_array().expect("Sync response should be an array");
        assert!(
            !items.is_empty(),
            "Sync response should contain at least 1 item"
        );

        let first = &items[0];
        assert_eq!(
            first["obj"].as_str().unwrap(),
            "/api/test-sync",
            "obj should match the synced path"
        );
        assert_eq!(
            first["act"].as_str().unwrap(),
            "GET",
            "act should match the synced method"
        );
    })
    .await;
}
