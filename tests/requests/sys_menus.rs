use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

const SYS_MENU_USER_MGMT_ID: &str = "cccccccc-cccc-cccc-cccc-ccccccccc006";

#[tokio::test]
#[serial]
async fn unauthenticated_list_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/sys-menus").await;
        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Unauthenticated request should return 401 or 403, got {status}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn non_super_admin_list_returns_error() {
    request::<App, _, _>(|request, ctx| async move {
        let user = prepare_data::init_user_login(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&user.token);
        let response = request
            .get("/api/sys-menus")
            .add_header(auth_key, auth_value)
            .await;
        assert_ne!(
            response.status_code(),
            200,
            "Non-super-admin should not access sys-menus"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_list_sys_menus() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);
        let response = request
            .get("/api/sys-menus")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(response.status_code(), 200, "List sys-menus should succeed");

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let items = body.as_array().expect("Response should be an array");
        assert!(
            items.len() >= 10,
            "Expected at least 10 sys-menus, got {}",
            items.len()
        );

        let first = &items[0];
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
        assert!(first.get("remark").is_some(), "Missing 'remark' key");
        assert!(first.get("status").is_some(), "Missing 'status' key");
        assert!(first.get("version").is_some(), "Missing 'version' key");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_sys_menu_tree() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);
        let response = request
            .get("/api/sys-menus/tree")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            response.status_code(),
            200,
            "Get sys-menu tree should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let roots = body.as_array().expect("Tree response should be an array");
        assert!(
            roots.len() >= 2,
            "Expected at least 2 root items (auth + system), got {}",
            roots.len()
        );

        // Each root should have a children array
        for root in roots {
            assert!(
                root.get("children").is_some(),
                "Root item should have 'children' field"
            );
        }

        // Find the "system" root and check its children
        let system_root = roots
            .iter()
            .find(|r| r.get("code").and_then(|c| c.as_str()) == Some("system"))
            .expect("Should find root with code == 'system'");
        let system_children = system_root["children"]
            .as_array()
            .expect("system root should have children array");
        assert!(
            system_children.len() >= 5,
            "system root should have at least 5 children, got {}",
            system_children.len()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_sys_menu() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let payload = serde_json::json!({
            "name": "Reports",
            "code": "reports",
            "type": "menu",
            "path": "/reports",
            "sortOrder": 10
        });

        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);
        let response = request
            .post("/api/sys-menus")
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            response.status_code(),
            200,
            "Create sys-menu should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["name"], "Reports");
        assert_eq!(body["code"], "reports");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_update_sys_menu() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let payload = serde_json::json!({
            "name": "用户管理V2",
            "version": 1
        });

        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/sys-menus/{SYS_MENU_USER_MGMT_ID}"))
            .json(&payload)
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            response.status_code(),
            200,
            "Update sys-menu should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["name"], "用户管理V2");
        assert_eq!(body["version"], 2);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_delete_sys_menu() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Create a new menu to delete (don't delete fixture data)
        let create_payload = serde_json::json!({
            "name": "Temp Menu",
            "code": "temp_menu_delete",
            "type": "menu",
            "path": "/temp-delete",
            "sortOrder": 99
        });

        let (k, v) = prepare_data::auth_header(&admin.token);
        let create_response = request
            .post("/api/sys-menus")
            .json(&create_payload)
            .add_header(k, v)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create temp sys-menu should succeed"
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let id = created["id"]
            .as_str()
            .expect("Created menu should have an 'id' field");

        // Delete the newly created menu
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let delete_response = request
            .delete(&format!("/api/sys-menus/{id}"))
            .add_header(k2, v2)
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "Delete sys-menu should succeed"
        );
    })
    .await;
}
