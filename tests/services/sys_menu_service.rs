use knota_fold::app::App;
use knota_fold::services::sys_menu_service;
use knota_fold::views::audit_logs::AuditContext;
use knota_fold::views::sys_menus::{CreateSysMenuRequest, UpdateSysMenuRequest};
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

const USER_ID: &str = "11111111-1111-1111-1111-111111111111";

fn test_audit_ctx() -> AuditContext {
    AuditContext {
        trace_id: Some("test-trace-id".to_string()),
        request_id: Some("test-request-id".to_string()),
        tenant_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        user_id: Some(Uuid::parse_str(USER_ID).unwrap()),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("test-agent".to_string()),
    }
}

#[tokio::test]
#[serial]
async fn can_list_sys_menus_flat() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let menus = sys_menu_service::list_sys_menus(db)
        .await
        .expect("Failed to list sys menus");
    // 10 seeded sys_menus; additive seed may add more
    assert!(menus.len() >= 10, "Expected at least 10 seeded sys_menus");
}

#[tokio::test]
#[serial]
async fn can_get_sys_menu_tree() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tree = sys_menu_service::get_sys_menu_tree(db)
        .await
        .expect("Failed to get sys menu tree");
    // 2 root directories: 权限管理 (auth), 系统管理 (system)
    assert!(tree.len() >= 2, "Expected at least 2 root menus");

    let system = tree
        .iter()
        .find(|m| m.code == "system")
        .expect("System menu not found");
    // system has 5 children: user_mgmt, tenant_mgmt, dict_mgmt, menu_mgmt, sys_menu_mgmt
    assert!(
        system.children.len() >= 5,
        "System should have at least 5 children"
    );
}

#[tokio::test]
#[serial]
async fn can_create_sys_menu() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let user = Uuid::parse_str(USER_ID).unwrap();

    let params = CreateSysMenuRequest {
        name: "Reports".to_string(),
        code: "reports".to_string(),
        menu_type: "menu".to_string(),
        path: Some("/reports".to_string()),
        alias: None,
        icon: None,
        parent_id: None,
        is_cache: None,
        sort_order: Some(10),
        remark: None,
    };

    let result = sys_menu_service::create_sys_menu(db, user, &params, &test_audit_ctx())
        .await
        .expect("Failed to create sys menu");
    assert_eq!(result.name, "Reports");
    assert_eq!(result.code, "reports");
}

#[tokio::test]
#[serial]
async fn can_update_sys_menu() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let user = Uuid::parse_str(USER_ID).unwrap();
    let menu_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc006").unwrap(); // 用户管理

    let params = UpdateSysMenuRequest {
        name: Some("用户管理V2".to_string()),
        code: None,
        menu_type: None,
        path: None,
        alias: None,
        icon: None,
        parent_id: None,
        is_cache: None,
        sort_order: None,
        remark: None,
        status: None,
        version: 1,
    };

    let result =
        sys_menu_service::update_sys_menu(db, menu_id, user, &params, &test_audit_ctx())
            .await
            .expect("Failed to update sys menu");
    assert_eq!(result.name, "用户管理V2");
    assert_eq!(result.version, 2);
}

#[tokio::test]
#[serial]
async fn update_sys_menu_version_conflict() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let user = Uuid::parse_str(USER_ID).unwrap();
    let menu_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc006").unwrap();

    let params = UpdateSysMenuRequest {
        name: Some("Conflict".to_string()),
        code: None,
        menu_type: None,
        path: None,
        alias: None,
        icon: None,
        parent_id: None,
        is_cache: None,
        sort_order: None,
        remark: None,
        status: None,
        version: 999, // wrong version
    };

    let result =
        sys_menu_service::update_sys_menu(db, menu_id, user, &params, &test_audit_ctx())
            .await;
    assert!(result.is_err(), "Expected version conflict error");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("Version conflict"),
        "Error should mention version conflict, got: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn can_soft_delete_sys_menu() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let target_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc006").unwrap(); // 用户管理

    let before = sys_menu_service::list_sys_menus(db)
        .await
        .expect("Failed to list before delete");
    let before_count = before.len();

    sys_menu_service::delete_sys_menu(db, target_id, &test_audit_ctx())
        .await
        .expect("Failed to delete sys menu");

    let after = sys_menu_service::list_sys_menus(db)
        .await
        .expect("Failed to list after delete");
    assert_eq!(
        after.len(),
        before_count - 1,
        "Expected one fewer menu after soft delete"
    );
}

#[tokio::test]
#[serial]
async fn sys_menu_code_must_be_unique() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let user = Uuid::parse_str(USER_ID).unwrap();

    // "system" code already exists in sys_menus
    let params = CreateSysMenuRequest {
        name: "Duplicate System".to_string(),
        code: "system".to_string(),
        menu_type: "directory".to_string(),
        path: None,
        alias: None,
        icon: None,
        parent_id: None,
        is_cache: None,
        sort_order: Some(99),
        remark: None,
    };

    let result =
        sys_menu_service::create_sys_menu(db, user, &params, &test_audit_ctx()).await;
    assert!(
        result.is_err(),
        "Should not allow duplicate code in sys_menus"
    );
}
