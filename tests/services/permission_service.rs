use knota_fold::app::App;
use knota_fold::services::permission_service;
use knota_fold::views::permissions::{CreatePermissionRequest, UpdatePermissionRequest};
use loco_rs::prelude::model::query::PaginationQuery;
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

const USER_ID: &str = "11111111-1111-1111-1111-111111111111";
const ROLE_READ_PERMISSION_ID: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb001";
const MENU_READ_PERMISSION_ID: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb003";

fn pagination(page: &str, page_size: &str) -> PaginationQuery {
    serde_json::from_value(serde_json::json!({
        "page_size": page_size,
        "page": page,
    }))
    .unwrap()
}

#[tokio::test]
#[serial]
async fn can_list_permissions_paginated() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let pagination = pagination("1", "10");

    let response = permission_service::list_permissions(db, &pagination)
        .await
        .expect("Failed to list permissions");

    // Fixtures contain 30 permissions, but prior tests may create additional records
    // since loco-rs seed() is additive and does not truncate test-created data
    assert!(response.total_items >= 30);
}

#[tokio::test]
#[serial]
async fn can_create_permission() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let params = CreatePermissionRequest {
        name: "Export Roles".to_string(),
        code: "role:export".to_string(),
        obj: "role".to_string(),
        act: "export".to_string(),
        permission_type: "api".to_string(),
        is_system: Some(false),
    };

    let created = permission_service::create_permission(db, user_id, &params)
        .await
        .expect("Failed to create permission");

    assert_eq!(created.name, "Export Roles");
    assert_eq!(created.code, "role:export");
    assert_eq!(created.obj, "role");
    assert_eq!(created.act, "export");
    assert_eq!(created.permission_type, "api");
    assert_eq!(created.version, 1);
}

#[tokio::test]
#[serial]
async fn can_update_permission() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let permission_id = Uuid::parse_str(ROLE_READ_PERMISSION_ID).unwrap();
    let params = UpdatePermissionRequest {
        name: Some("Read Roles Updated".to_string()),
        code: None,
        obj: None,
        act: None,
        permission_type: None,
        is_system: None,
        version: 1,
    };

    let updated =
        permission_service::update_permission(db, permission_id, user_id, &params)
            .await
            .expect("Failed to update permission");

    assert_eq!(updated.name, "Read Roles Updated");
    assert_eq!(updated.version, 2);
}

#[tokio::test]
#[serial]
async fn update_permission_version_conflict() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(USER_ID).unwrap();
    let permission_id = Uuid::parse_str(ROLE_READ_PERMISSION_ID).unwrap();
    let params = UpdatePermissionRequest {
        name: Some("Conflicted Permission".to_string()),
        code: None,
        obj: None,
        act: None,
        permission_type: None,
        is_system: None,
        version: 999,
    };

    let err = permission_service::update_permission(db, permission_id, user_id, &params)
        .await
        .expect_err("Expected version conflict");

    assert!(
        format!("{err:?}").contains("Version conflict"),
        "Error should mention version conflict, got: {err:?}"
    );
}

#[tokio::test]
#[serial]
async fn can_soft_delete_permission() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let permission_id = Uuid::parse_str(MENU_READ_PERMISSION_ID).unwrap();
    let pagination = pagination("1", "10");

    knota_fold::models::permissions::Model::soft_delete(db, permission_id)
        .await
        .expect("Failed to soft delete permission");

    let response = permission_service::list_permissions(db, &pagination)
        .await
        .expect("Failed to list permissions after soft delete");

    // After soft-deleting one permission, total should be at least 29
    assert!(response.total_items >= 29);
    assert!(!response
        .items
        .iter()
        .any(|permission| permission.id == permission_id.to_string()));
}

#[tokio::test]
#[serial]
async fn permission_code_must_be_globally_unique() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(USER_ID).unwrap();

    // "GET:/api/permissions" already exists in fixtures. Creating another with the same code
    // should fail due to the global unique constraint on code.
    let params = CreatePermissionRequest {
        name: "Duplicate Code".to_string(),
        code: "GET:/api/permissions".to_string(),
        obj: "/api/permissions".to_string(),
        act: "GET".to_string(),
        permission_type: "api".to_string(),
        is_system: Some(false),
    };

    let result = permission_service::create_permission(db, user_id, &params).await;
    assert!(
        result.is_err(),
        "Should not allow duplicate permission code globally"
    );
}

/// Sync → soft-delete → re-sync should restore the soft-deleted record
/// instead of failing with a unique constraint violation.
#[tokio::test]
#[serial]
async fn sync_delete_resync_restores_soft_deleted_permission() {
    use knota_fold::views::permissions::SyncPermissionItem;

    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = Uuid::parse_str(USER_ID).unwrap();

    // 1. Sync a unique permission
    let items = vec![SyncPermissionItem {
        path: "/api/test-upsert-unique".to_string(),
        method: "POST".to_string(),
    }];

    let created = permission_service::sync_permissions(db, user_id, &items)
        .await
        .expect("First sync should succeed");
    assert_eq!(created.len(), 1);
    let original_id = created[0].id;
    assert!(created[0].deleted_at.is_none());

    // 2. Soft-delete it
    knota_fold::models::permissions::Model::soft_delete(db, original_id)
        .await
        .expect("Soft delete should succeed");

    // Verify it's no longer in active list
    let active = knota_fold::models::permissions::Model::find_all(db)
        .await
        .expect("find_all should succeed");
    assert!(
        !active.iter().any(|p| p.id == original_id),
        "Deleted permission should not appear in active list"
    );

    // 3. Re-sync the same permission — should restore, not fail
    let re_synced = permission_service::sync_permissions(db, user_id, &items)
        .await
        .expect(
            "Re-sync should succeed (upsert), not fail with unique constraint violation",
        );
    assert_eq!(re_synced.len(), 1);
    // Restored record should keep the same ID
    assert_eq!(re_synced[0].id, original_id);
    assert!(re_synced[0].deleted_at.is_none());
    assert_eq!(re_synced[0].code, "POST:/api/test-upsert-unique");

    // 4. Third sync — already active, should skip
    let third = permission_service::sync_permissions(db, user_id, &items)
        .await
        .expect("Third sync should succeed");
    assert_eq!(
        third.len(),
        0,
        "Already active permission should be skipped"
    );
}
