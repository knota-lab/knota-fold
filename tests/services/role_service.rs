use knota_fold::app::App;
use knota_fold::services::role_service;
use knota_fold::views::audit_logs::AuditContext;
use knota_fold::views::roles::{CreateRoleRequest, RoleListParams, UpdateRoleRequest};
use loco_rs::prelude::model::query::PaginationQuery;
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

const TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";
const USER_ID: &str = "11111111-1111-1111-1111-111111111111";
const SUPER_ADMIN_ROLE_ID: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaa001";
const TENANT_ADMIN_ROLE_ID: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaa004";

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

fn pagination(page: &str, page_size: &str) -> PaginationQuery {
    serde_json::from_value(serde_json::json!({
        "page_size": page_size,
        "page": page,
    }))
    .unwrap()
}

fn default_search() -> RoleListParams {
    RoleListParams {
        page: 1,
        page_size: 10,
        tenant_code: None,
        name: None,
        status: None,
    }
}

#[tokio::test]
#[serial]
async fn can_list_roles_paginated() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let pagination = pagination("1", "2");

    let response =
        role_service::list_roles(db, Some(tenant_id), &pagination, &default_search())
            .await
            .expect("Failed to list roles");

    assert!(!response.items.is_empty(), "Should return at least 1 role");
    assert!(response.total_items >= 2, "Should have at least 2 roles");
}

#[tokio::test]
#[serial]
async fn can_create_role() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let params = CreateRoleRequest {
        name: "Auditor".to_string(),
        code: "AUDITOR".to_string(),
        parent_id: None,
        is_system: Some(false),
        description: Some("Audit role".to_string()),
    };

    let created =
        role_service::create_role(db, tenant_id, user_id, &params, &test_audit_ctx())
            .await
            .expect("Failed to create role");

    assert_eq!(created.name, "Auditor");
    assert_eq!(created.code, "AUDITOR");
    assert_eq!(created.tenant_id, tenant_id);
    assert_eq!(created.updated_by, Some(user_id));
    assert_eq!(created.version, 1);
}

#[tokio::test]
#[serial]
async fn can_update_role() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let user_id = Uuid::parse_str(USER_ID).unwrap();
    // Create a test role first so we're not dependent on fixture UUIDs
    let create_params = CreateRoleRequest {
        name: "Test Update Target".to_string(),
        code: "TEST_UPDATE_TARGET".to_string(),
        parent_id: None,
        is_system: Some(false),
        description: None,
    };
    let created = role_service::create_role(
        db,
        tenant_id,
        user_id,
        &create_params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create role for update test");
    assert_eq!(created.version, 1);

    let params = UpdateRoleRequest {
        name: Some("Renamed Role".to_string()),
        code: None,
        parent_id: None,
        is_system: None,
        description: None,
        version: 1,
    };

    let updated = role_service::update_role(
        db,
        created.id,
        tenant_id,
        user_id,
        &params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to update role");

    assert_eq!(updated.name, "Renamed Role");
    assert_eq!(updated.version, 2);
}

#[tokio::test]
#[serial]
async fn update_role_version_conflict() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let create_params = CreateRoleRequest {
        name: "Version Conflict Test".to_string(),
        code: "TEST_VERSION_CONFLICT".to_string(),
        parent_id: None,
        is_system: Some(false),
        description: None,
    };
    let created = role_service::create_role(
        db,
        tenant_id,
        user_id,
        &create_params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create role for version conflict test");

    let params = UpdateRoleRequest {
        name: Some("Conflicted Role".to_string()),
        code: None,
        parent_id: None,
        is_system: None,
        description: None,
        version: 999,
    };

    role_service::update_role(
        db,
        created.id,
        tenant_id,
        user_id,
        &params,
        &test_audit_ctx(),
    )
    .await
    .expect_err("Expected version conflict");
}

#[tokio::test]
#[serial]
async fn can_toggle_role_status_without_casbin() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let pagination = pagination("1", "10");

    // Create a non-protected role that can be freely toggled
    let create_params = CreateRoleRequest {
        name: "Toggle Test Role".to_string(),
        code: "TEST_TOGGLE".to_string(),
        parent_id: None,
        is_system: Some(false),
        description: None,
    };
    let created = role_service::create_role(
        db,
        tenant_id,
        user_id,
        &create_params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create role for toggle test");

    knota_fold::models::roles::Model::toggle_status(
        db, created.id, tenant_id, "disabled",
    )
    .await
    .expect("Failed to disable role");

    // Disabled role should still appear in the list (list shows all)
    let response =
        role_service::list_roles(db, Some(tenant_id), &pagination, &default_search())
            .await
            .expect("Failed to list roles after disable");

    assert!(
        response.total_items >= 3,
        "Should have at least 3 roles (2 fixture + 1 created)"
    );
    let disabled_role = response
        .items
        .iter()
        .find(|role| role.id == created.id.to_string());
    assert!(
        disabled_role.is_some(),
        "Disabled role should still appear in list"
    );
}

#[tokio::test]
#[serial]
async fn can_list_roles_second_page() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let pagination = pagination("2", "2");

    let response =
        role_service::list_roles(db, Some(tenant_id), &pagination, &default_search())
            .await
            .expect("Failed to list roles");

    // With total_items=2 and page_size=2, page 2 should be empty
    assert!(
        response.items.is_empty(),
        "Second page should be empty when total fits on page 1"
    );
}

const OTHER_TENANT_ID: &str = "00000000-0000-0000-0000-000000000002";

#[tokio::test]
#[serial]
async fn role_cannot_access_other_tenant_data() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_b = Uuid::parse_str(OTHER_TENANT_ID).unwrap();
    let user_b = Uuid::parse_str(USER_ID).unwrap();
    let role_id_a = Uuid::parse_str(SUPER_ADMIN_ROLE_ID).unwrap();

    // 1. Try to update Tenant A's role using Tenant B's context
    let params = UpdateRoleRequest {
        name: Some("Hacked Role".to_string()),
        code: None,
        parent_id: None,
        is_system: None,
        description: None,
        version: 1,
    };
    let result = role_service::update_role(
        db,
        role_id_a,
        tenant_b,
        user_b,
        &params,
        &test_audit_ctx(),
    )
    .await;
    assert!(
        result.is_err(),
        "Tenant B should not be able to update Tenant A's role"
    );

    // 2. Try to list roles using Tenant B's context - should be empty initially
    let pagination = pagination("1", "10");
    let response =
        role_service::list_roles(db, Some(tenant_b), &pagination, &default_search())
            .await
            .unwrap();
    assert_eq!(
        response.total_items, 0,
        "Tenant B should not see Tenant A's roles"
    );
}

#[tokio::test]
#[serial]
async fn role_can_have_duplicate_code_in_different_tenants() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_b = Uuid::parse_str(OTHER_TENANT_ID).unwrap();
    let user_b = Uuid::parse_str(USER_ID).unwrap();

    // "ADMIN" already exists in Tenant A. We should be able to create it in Tenant B.
    let params = CreateRoleRequest {
        name: "Admin for B".to_string(),
        code: "ADMIN".to_string(),
        parent_id: None,
        is_system: Some(false),
        description: None,
    };

    let result =
        role_service::create_role(db, tenant_b, user_b, &params, &test_audit_ctx()).await;
    assert!(
        result.is_ok(),
        "Should allow duplicate role code in different tenant"
    );
}

#[tokio::test]
#[serial]
async fn can_get_role_permission_ids() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let role_id = Uuid::parse_str(SUPER_ADMIN_ROLE_ID).unwrap();

    let permission_ids = role_service::get_role_permission_ids(db, role_id, tenant_id)
        .await
        .expect("Failed to get role permission ids");

    // SUPER_ADMIN has various permissions in fixtures (route-based format).
    // Use >= 1 since exact count varies with fixture changes.
    assert!(
        !permission_ids.is_empty(),
        "SUPER_ADMIN should have permissions"
    );
}

#[tokio::test]
#[serial]
async fn get_role_permission_ids_empty_for_role_without_permissions() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let empty_role = role_service::create_role(
        db,
        tenant_id,
        user_id,
        &CreateRoleRequest {
            name: "No Permissions".to_string(),
            code: "NO_PERMS".to_string(),
            parent_id: None,
            is_system: Some(false),
            description: None,
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create role without permissions");

    let permission_ids =
        role_service::get_role_permission_ids(db, empty_role.id, tenant_id)
            .await
            .expect("Failed to get role permission ids");

    // Freshly created role has no permission assignments.
    assert!(permission_ids.is_empty());
}

#[tokio::test]
#[serial]
async fn can_sync_and_get_role_menu_ids() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let role_id = Uuid::parse_str(TENANT_ADMIN_ROLE_ID).unwrap();

    let menu_id_1 = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc001").unwrap();
    let menu_id_2 = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc002").unwrap();

    // Sync two menu ids to the role
    role_service::sync_role_menus(
        db,
        tenant_id,
        role_id,
        vec![menu_id_1, menu_id_2],
        Uuid::parse_str(USER_ID).unwrap(),
        true,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to sync role menus");

    // Retrieve and verify
    let ids = role_service::get_role_menu_ids(db, role_id, tenant_id)
        .await
        .expect("Failed to get role menu ids");

    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&menu_id_1.to_string()));
    assert!(ids.contains(&menu_id_2.to_string()));
}

#[tokio::test]
#[serial]
async fn sync_role_menus_replaces_previous() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let role_id = Uuid::parse_str(TENANT_ADMIN_ROLE_ID).unwrap();

    let menu_id_1 = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc001").unwrap();
    let menu_id_3 = Uuid::parse_str("cccccccc-cccc-cccc-cccc-ccccccccc003").unwrap();

    // First sync: set menu_id_1
    role_service::sync_role_menus(
        db,
        tenant_id,
        role_id,
        vec![menu_id_1],
        Uuid::parse_str(USER_ID).unwrap(),
        true,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed first sync");

    // Second sync: replace with menu_id_3
    role_service::sync_role_menus(
        db,
        tenant_id,
        role_id,
        vec![menu_id_3],
        Uuid::parse_str(USER_ID).unwrap(),
        true,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed second sync");

    let ids = role_service::get_role_menu_ids(db, role_id, tenant_id)
        .await
        .expect("Failed to get role menu ids");

    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&menu_id_3.to_string()));
    assert!(
        !ids.contains(&menu_id_1.to_string()),
        "Old menu should be removed after re-sync"
    );
}

#[tokio::test]
#[serial]
async fn get_role_menu_ids_empty_for_role_without_menus() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let role_id = Uuid::parse_str(TENANT_ADMIN_ROLE_ID).unwrap();

    let ids = role_service::get_role_menu_ids(db, role_id, tenant_id)
        .await
        .expect("Failed to get role menu ids");

    // USER role has no menu assignments in fixtures
    assert!(ids.is_empty());
}

#[tokio::test]
#[serial]
async fn cannot_disable_super_admin_role() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let role_id = Uuid::parse_str(SUPER_ADMIN_ROLE_ID).unwrap();

    let enforcer = knota_fold::services::casbin_service::init_enforcer(db)
        .await
        .expect("Failed to init enforcer");

    let err = role_service::toggle_role_status(
        db,
        &enforcer,
        role_id,
        tenant_id,
        "disabled",
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject disabling SUPER_ADMIN");

    assert!(
        format!("{err:?}").contains("保护") || format!("{err:?}").contains("protect"),
        "Error should mention protected role, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn cannot_disable_tenant_admin_role() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let role_id = Uuid::parse_str(TENANT_ADMIN_ROLE_ID).unwrap();

    let enforcer = knota_fold::services::casbin_service::init_enforcer(db)
        .await
        .expect("Failed to init enforcer");

    let err = role_service::toggle_role_status(
        db,
        &enforcer,
        role_id,
        tenant_id,
        "disabled",
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject disabling TENANT_ADMIN");

    assert!(
        format!("{err:?}").contains("保护") || format!("{err:?}").contains("protect"),
        "Error should mention protected role, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn can_disable_regular_role_via_service() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let user_id = Uuid::parse_str(USER_ID).unwrap();

    // Create a non-protected role that can be disabled
    let create_params = CreateRoleRequest {
        name: "Disable Test Role".to_string(),
        code: "TEST_DISABLE".to_string(),
        parent_id: None,
        is_system: Some(false),
        description: None,
    };
    let created = role_service::create_role(
        db,
        tenant_id,
        user_id,
        &create_params,
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create role for disable test");

    let enforcer = knota_fold::services::casbin_service::init_enforcer(db)
        .await
        .expect("Failed to init enforcer");

    let disabled = role_service::toggle_role_status(
        db,
        &enforcer,
        created.id,
        tenant_id,
        "disabled",
        &test_audit_ctx(),
    )
    .await
    .expect("Should allow disabling non-protected role");

    assert_eq!(disabled.status.as_str(), "disabled");
}
