use knota_fold::app::App;
use knota_fold::services::i18n_service;
use knota_fold::views::audit_logs::AuditContext;
use knota_fold::views::i18n::ImportEntry;
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

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap()
}

/// Helper: insert a global translation row directly via the upsert API.
async fn insert_global_translation(
    ctx: &loco_rs::prelude::AppContext,
    namespace: &str,
    key: &str,
    locale: &str,
    value: &str,
) {
    use knota_fold::views::i18n::UpsertGlobalTranslationRequest;
    i18n_service::upsert_global_translation(
        ctx,
        uuid(USER_ID),
        &UpsertGlobalTranslationRequest {
            namespace: namespace.to_string(),
            key: key.to_string(),
            locale: locale.to_string(),
            value: value.to_string(),
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to insert global translation");
}

fn entry(namespace: &str, key: &str, locale: &str, value: &str) -> ImportEntry {
    ImportEntry {
        namespace: namespace.to_string(),
        key: key.to_string(),
        locale: locale.to_string(),
        value: value.to_string(),
    }
}

// ── batch_update_global tests ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn batch_update_updates_existing_rows() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let ctx = &boot.app_context;

    // Prepare: insert two translations.
    insert_global_translation(ctx, "TestNs", "key1", "zh-CN", "旧值1").await;
    insert_global_translation(ctx, "TestNs", "key2", "zh-CN", "旧值2").await;

    let resp = i18n_service::batch_update_global(
        ctx,
        uuid(USER_ID),
        &[
            entry("TestNs", "key1", "zh-CN", "新值1"),
            entry("TestNs", "key2", "zh-CN", "新值2"),
        ],
        &test_audit_ctx(),
    )
    .await
    .expect("batch_update_global should succeed");

    assert_eq!(resp.updated, 2);
    assert_eq!(resp.inserted, 0);
    assert_eq!(resp.skipped, 0);
}

#[tokio::test]
#[serial]
async fn batch_update_skips_nonexistent_rows() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let ctx = &boot.app_context;

    // Prepare: insert only one translation.
    insert_global_translation(ctx, "TestNs", "exists", "zh-CN", "已存在").await;

    let resp = i18n_service::batch_update_global(
        ctx,
        uuid(USER_ID),
        &[
            entry("TestNs", "exists", "zh-CN", "更新值"),
            entry("FakeNamespace", "nope", "xx-XX", "不存在"),
            entry("TestNs", "nope", "zh-CN", "也不存在"),
        ],
        &test_audit_ctx(),
    )
    .await
    .expect("batch_update_global should succeed");

    assert_eq!(resp.updated, 1);
    assert_eq!(resp.inserted, 0);
    assert_eq!(resp.skipped, 2);
}

#[tokio::test]
#[serial]
async fn batch_update_never_creates_new_rows() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let ctx = &boot.app_context;

    let resp = i18n_service::batch_update_global(
        ctx,
        uuid(USER_ID),
        &[entry("FakeNs", "fakeKey", "xx-XX", "垃圾数据")],
        &test_audit_ctx(),
    )
    .await
    .expect("batch_update_global should succeed");

    assert_eq!(resp.updated, 0);
    assert_eq!(resp.inserted, 0);
    assert_eq!(resp.skipped, 1);

    // Verify the row was NOT created in DB.
    let keys =
        i18n_service::list_global_keys(&ctx.db, Some("FakeNs"), None, None, 1, 100)
            .await
            .expect("list_global_keys should succeed");
    assert_eq!(
        keys.total_items, 0,
        "batch_update should NOT create new rows"
    );
}

#[tokio::test]
#[serial]
async fn batch_update_rejects_empty_entries() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");

    let err = i18n_service::batch_update_global(
        &boot.app_context,
        uuid(USER_ID),
        &[],
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject empty entries");

    assert!(
        format!("{err:?}").contains("entries 不能为空"),
        "Error should mention empty entries, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn batch_update_rejects_empty_value() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");

    let err = i18n_service::batch_update_global(
        &boot.app_context,
        uuid(USER_ID),
        &[entry("TestNs", "key1", "zh-CN", "")],
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject empty value");

    assert!(
        format!("{err:?}").contains("value 不能为空"),
        "Error should mention empty value, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn batch_update_rejects_invalid_locale_format() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");

    let err = i18n_service::batch_update_global(
        &boot.app_context,
        uuid(USER_ID),
        &[entry("TestNs", "key1", "INVALID!LOCALE", "值")],
        &test_audit_ctx(),
    )
    .await
    .expect_err("Should reject invalid locale format");

    assert!(
        format!("{err:?}").contains("locale"),
        "Error should mention locale, got: {err:?}"
    );
}

// ── batch_update_tenant tests ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn batch_update_tenant_skips_nonexistent() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let ctx = &boot.app_context;
    let tenant_id = uuid(TENANT_ID);

    // No tenant override exists for this (ns, key, locale), so all should be skipped.
    let resp = i18n_service::batch_update_tenant(
        ctx,
        tenant_id,
        uuid(USER_ID),
        &[entry("Tenant.DEFAULT", "key1", "zh-CN", "租户值")],
        &test_audit_ctx(),
    )
    .await
    .expect("batch_update_tenant should succeed");

    assert_eq!(resp.updated, 0);
    assert_eq!(resp.skipped, 1);
}

#[tokio::test]
#[serial]
async fn batch_update_tenant_updates_existing_override() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let ctx = &boot.app_context;
    let tenant_id = uuid(TENANT_ID);

    // Prepare: create a global translation and a tenant override.
    insert_global_translation(ctx, "Tenant.DEFAULT", "key1", "zh-CN", "全局值").await;

    use knota_fold::views::i18n::UpsertTenantOverrideRequest;
    i18n_service::upsert_tenant_override(
        ctx,
        tenant_id,
        uuid(USER_ID),
        &UpsertTenantOverrideRequest {
            namespace: "Tenant.DEFAULT".to_string(),
            key: "key1".to_string(),
            locale: "zh-CN".to_string(),
            value: "旧租户值".to_string(),
        },
        &test_audit_ctx(),
    )
    .await
    .expect("Failed to create tenant override");

    let resp = i18n_service::batch_update_tenant(
        ctx,
        tenant_id,
        uuid(USER_ID),
        &[entry("Tenant.DEFAULT", "key1", "zh-CN", "新租户值")],
        &test_audit_ctx(),
    )
    .await
    .expect("batch_update_tenant should succeed");

    assert_eq!(resp.updated, 1);
    assert_eq!(resp.skipped, 0);
}
