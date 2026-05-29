use knota_fold::app::App;
use knota_fold::services::tenant_menu_service;
use knota_fold::views::audit_logs::AuditContext;
use knota_fold::views::menus::UpdateOverrideRequest;
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

const TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";
const USER_ID: &str = "11111111-1111-1111-1111-111111111111";

fn test_audit_ctx() -> AuditContext {
    AuditContext {
        trace_id: Some("test-trace-id".to_string()),
        request_id: Some("test-request-id".to_string()),
        tenant_id: Uuid::parse_str(TENANT_ID).unwrap(),
        user_id: Some(Uuid::parse_str(USER_ID).unwrap()),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("test-agent".to_string()),
    }
}

#[tokio::test]
#[serial]
async fn can_get_merged_menu_tree() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant = Uuid::parse_str(TENANT_ID).unwrap();

    let tree = tenant_menu_service::get_merged_menu_tree(db, tenant)
        .await
        .expect("Failed to get merged menu tree");
    // With no overrides, merged tree should match sys_menus tree: 2 root directories
    assert!(tree.len() >= 2, "Expected at least 2 root menus");

    let system = tree
        .iter()
        .find(|m| m.code == "system")
        .expect("System menu not found in merged tree");
    assert!(
        system.children.len() >= 5,
        "System should have at least 5 children in merged tree"
    );
}

#[tokio::test]
#[serial]
async fn can_upsert_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant = Uuid::parse_str(TENANT_ID).unwrap();
    let user = Uuid::parse_str(USER_ID).unwrap();
    let menu_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc006").unwrap(); // 用户管理

    let params = UpdateOverrideRequest {
        custom_name: Some("用户管理(自定义)".to_string()),
        custom_icon: None,
        custom_sort: Some(99),
        is_hidden: None,
    };

    tenant_menu_service::upsert_override(
        db,
        tenant,
        menu_id,
        user,
        &params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to upsert override");

    // Verify override is reflected in merged tree
    let tree = tenant_menu_service::get_merged_menu_tree(db, tenant)
        .await
        .expect("Failed to get merged tree after override");

    let system = tree
        .iter()
        .find(|m| m.code == "system")
        .expect("System menu not found");

    let user_mgmt = system
        .children
        .iter()
        .find(|m| m.code == "user_mgmt")
        .expect("user_mgmt not found in children");

    assert_eq!(user_mgmt.name, "用户管理(自定义)");
    assert_eq!(user_mgmt.sort_order, 99);
}

#[tokio::test]
#[serial]
async fn can_delete_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant = Uuid::parse_str(TENANT_ID).unwrap();
    let user = Uuid::parse_str(USER_ID).unwrap();
    let menu_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc006").unwrap();

    // First create an override
    let params = UpdateOverrideRequest {
        custom_name: Some("Overridden".to_string()),
        custom_icon: None,
        custom_sort: None,
        is_hidden: None,
    };
    tenant_menu_service::upsert_override(
        db,
        tenant,
        menu_id,
        user,
        &params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to upsert override");

    // Then delete it
    tenant_menu_service::delete_override(db, tenant, menu_id, &test_audit_ctx())
        .await
        .expect("Failed to delete override");

    // Verify the menu reverts to platform default
    let tree = tenant_menu_service::get_merged_menu_tree(db, tenant)
        .await
        .expect("Failed to get merged tree after delete override");

    let system = tree
        .iter()
        .find(|m| m.code == "system")
        .expect("System menu not found");

    let user_mgmt = system
        .children
        .iter()
        .find(|m| m.code == "user_mgmt")
        .expect("user_mgmt not found");

    // Should revert to the original name from sys_menus fixture
    assert_eq!(user_mgmt.name, "用户管理");
}

#[tokio::test]
#[serial]
async fn can_hide_menu_via_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant = Uuid::parse_str(TENANT_ID).unwrap();
    let user = Uuid::parse_str(USER_ID).unwrap();
    let menu_id = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc006").unwrap();

    let params = UpdateOverrideRequest {
        custom_name: None,
        custom_icon: None,
        custom_sort: None,
        is_hidden: Some(true),
    };

    tenant_menu_service::upsert_override(
        db,
        tenant,
        menu_id,
        user,
        &params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to hide menu");

    // Hidden menus should be excluded from get_user_menus for non-super-admin
    // but get_merged_menu_tree still returns all (it's the admin view)
    let tree = tenant_menu_service::get_merged_menu_tree(db, tenant)
        .await
        .expect("Failed to get merged tree");

    // Merged tree still includes hidden items (admin view)
    let system = tree
        .iter()
        .find(|m| m.code == "system")
        .expect("System menu not found");
    assert!(
        system.children.iter().any(|m| m.code == "user_mgmt"),
        "Hidden menu should still appear in merged tree (admin view)"
    );
}

#[tokio::test]
#[serial]
async fn super_admin_gets_all_menus() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;
    let tenant = Uuid::parse_str(TENANT_ID).unwrap();
    let user = Uuid::parse_str(USER_ID).unwrap();

    let tree = tenant_menu_service::get_user_menus(db, user, tenant, true)
        .await
        .expect("Failed to get user menus for super admin");
    // Super admin skips layers 3&4, gets all menus as tree
    assert!(
        tree.len() >= 2,
        "Super admin should see at least 2 root menus"
    );
}
