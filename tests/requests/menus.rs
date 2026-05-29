use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

const SYS_MENU_USER_MGMT_ID: &str = "cccccccc-cccc-cccc-cccc-ccccccccc006";

#[tokio::test]
#[serial]
async fn unauthenticated_tree_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/menus/tree").await;
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
async fn can_get_menu_tree() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/menus/tree")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "GET /api/menus/tree should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let roots = body.as_array().expect("Response should be an array");
        assert!(
            roots.len() >= 2,
            "Expected at least 2 root menu items, got {}",
            roots.len()
        );

        // Verify camelCase keys on the first root item
        let first = &roots[0];
        assert!(first.get("id").is_some(), "Missing 'id' key");
        assert!(first.get("parentId").is_some(), "Missing 'parentId' key");
        assert!(first.get("code").is_some(), "Missing 'code' key");
        assert!(first.get("name").is_some(), "Missing 'name' key");
        assert!(first.get("path").is_some(), "Missing 'path' key");
        assert!(first.get("alias").is_some(), "Missing 'alias' key");
        assert!(first.get("icon").is_some(), "Missing 'icon' key");
        assert!(first.get("type").is_some(), "Missing 'type' key");
        assert!(first.get("isCache").is_some(), "Missing 'isCache' key");
        assert!(first.get("sortOrder").is_some(), "Missing 'sortOrder' key");
        assert!(first.get("children").is_some(), "Missing 'children' key");

        // Find the "system" root and verify its children
        let system_root = roots
            .iter()
            .find(|r| r["code"].as_str() == Some("system"))
            .expect("Expected a root menu with code 'system'");

        let children = system_root["children"]
            .as_array()
            .expect("system root should have 'children' array");
        assert!(
            children.len() >= 5,
            "Expected 'system' root to have at least 5 children, got {}",
            children.len()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_upsert_and_delete_override() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // 1. PUT override
        let override_payload = serde_json::json!({
            "customName": "用户管理(自定义)",
            "customSort": 99
        });

        let (k, v) = prepare_data::auth_header(&admin.token);
        let put_response = request
            .put(&format!("/api/menus/{SYS_MENU_USER_MGMT_ID}/override"))
            .json(&override_payload)
            .add_header(k, v)
            .await;

        assert_eq!(
            put_response.status_code(),
            200,
            "PUT override should succeed"
        );

        // 2. GET tree → verify override applied
        let (k, v) = prepare_data::auth_header(&admin.token);
        let tree_response = request.get("/api/menus/tree").add_header(k, v).await;

        assert_eq!(
            tree_response.status_code(),
            200,
            "GET tree should succeed after override"
        );

        let body: serde_json::Value =
            serde_json::from_str(&tree_response.text()).unwrap();
        let roots = body.as_array().unwrap();
        let system_root = roots
            .iter()
            .find(|r| r["code"].as_str() == Some("system"))
            .expect("Expected 'system' root");

        let children = system_root["children"].as_array().unwrap();
        let user_mgmt = children
            .iter()
            .find(|c| c["id"].as_str() == Some(SYS_MENU_USER_MGMT_ID))
            .expect("Expected user_mgmt child under system");

        assert_eq!(
            user_mgmt["name"].as_str().unwrap(),
            "用户管理(自定义)",
            "Override name should be applied"
        );
        assert_eq!(
            user_mgmt["sortOrder"].as_i64().unwrap(),
            99,
            "Override sort order should be applied"
        );

        // 3. DELETE override
        let (k, v) = prepare_data::auth_header(&admin.token);
        let delete_response = request
            .delete(&format!("/api/menus/{SYS_MENU_USER_MGMT_ID}/override"))
            .add_header(k, v)
            .await;

        assert_eq!(
            delete_response.status_code(),
            200,
            "DELETE override should succeed"
        );

        // 4. GET tree → verify reverted
        let (k, v) = prepare_data::auth_header(&admin.token);
        let tree_response = request.get("/api/menus/tree").add_header(k, v).await;

        assert_eq!(
            tree_response.status_code(),
            200,
            "GET tree should succeed after delete"
        );

        let body: serde_json::Value =
            serde_json::from_str(&tree_response.text()).unwrap();
        let roots = body.as_array().unwrap();
        let system_root = roots
            .iter()
            .find(|r| r["code"].as_str() == Some("system"))
            .expect("Expected 'system' root after revert");

        let children = system_root["children"].as_array().unwrap();
        let user_mgmt = children
            .iter()
            .find(|c| c["id"].as_str() == Some(SYS_MENU_USER_MGMT_ID))
            .expect("Expected user_mgmt child after revert");

        assert_eq!(
            user_mgmt["name"].as_str().unwrap(),
            "用户管理",
            "Name should revert to original after override deleted"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_my_menus() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/users/me/menus")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "GET /api/users/me/menus should succeed for super admin"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let roots = body.as_array().expect("Response should be an array");
        assert!(
            roots.len() >= 2,
            "Super admin should see at least 2 root menu items, got {}",
            roots.len()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn non_super_admin_can_get_my_menus() {
    request::<App, _, _>(|request, ctx| async move {
        let user = prepare_data::init_user_login(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&user.token);

        let response = request
            .get("/api/users/me/menus")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "GET /api/users/me/menus should succeed for non-super-admin (whitelisted path)"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        // Fresh user with no roles — body is an array (possibly empty)
        assert!(body.is_array(), "Response should be an array");
    })
    .await;
}
