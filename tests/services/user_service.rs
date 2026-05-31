use knota_fold::app::App;
use knota_fold::services::user_service;
use knota_fold::views::audit_logs::AuditContext;
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

const TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";
const SUPER_ADMIN_USER_ID: &str = "00000000-0000-0000-0000-000000000099";
const USER1_ID: &str = "11111111-1111-1111-1111-111111111111";

fn test_audit_ctx() -> AuditContext {
    AuditContext {
        trace_id: Some("test-trace-id".to_string()),
        request_id: Some("test-request-id".to_string()),
        tenant_id: Uuid::parse_str(TENANT_ID).unwrap(),
        user_id: Some(Uuid::parse_str(SUPER_ADMIN_USER_ID).unwrap()),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("test-agent".to_string()),
    }
}

#[tokio::test]
#[serial]
async fn super_admin_cannot_disable_self() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(SUPER_ADMIN_USER_ID).unwrap();
    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();

    let err = user_service::toggle_user_status(
        db,
        user_id,
        tenant_id,
        user_id,
        "disabled",
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject super admin disabling themselves");

    assert!(
        format!("{err:?}").contains("管理员不能禁用自己的帐户"),
        "Error should mention admin self-disable, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn cannot_disable_last_super_admin() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let super_admin_id = Uuid::parse_str(SUPER_ADMIN_USER_ID).unwrap();
    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    // Use a different caller (user1) to avoid hitting the self-disable guard
    let caller_id = Uuid::parse_str(USER1_ID).unwrap();

    let err = user_service::toggle_user_status(
        db,
        super_admin_id,
        tenant_id,
        caller_id,
        "disabled",
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject disabling the last super admin");

    assert!(
        format!("{err:?}").contains("系统中仅剩一个超级管理员"),
        "Error should mention last super admin, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn can_disable_regular_user() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(USER1_ID).unwrap();
    let tenant_id = Uuid::parse_str(TENANT_ID).unwrap();
    let caller_id = Uuid::parse_str(SUPER_ADMIN_USER_ID).unwrap();

    let result = user_service::toggle_user_status(
        db,
        user_id,
        tenant_id,
        caller_id,
        "disabled",
        &test_audit_ctx(),
    )
    .await;

    assert!(result.is_ok(), "Should allow disabling a regular user");
    assert_eq!(result.unwrap().status, "disabled");
}
