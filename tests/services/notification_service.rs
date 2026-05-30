use knota_fold::app::App;
use knota_fold::models::users;
use loco_rs::testing::prelude::*;
use serial_test::serial;
use uuid::Uuid;

const SUPER_ADMIN_EMAIL: &str = "super.admin@knota.com";

async fn get_super_admin_id(db: &sea_orm::DatabaseConnection) -> Uuid {
    let user = users::Model::find_by_email(db, SUPER_ADMIN_EMAIL)
        .await
        .expect("super admin should exist in seed data");
    user.id
}

async fn get_default_tenant_id(db: &sea_orm::DatabaseConnection) -> Uuid {
    let user = users::Model::find_by_email(db, SUPER_ADMIN_EMAIL)
        .await
        .expect("super admin should exist");
    user.tenant_id
}

#[tokio::test]
#[serial]
async fn can_create_platform_notification() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;

    let notification =
        knota_fold::modules::notification::service::create::create_notification(
            db,
            &knota_fold::modules::notification::service::create::CreateNotificationParams {
                tenant_id: None,
                created_by: user_id,
                title: "Platform Notice",
                content: "Hello everyone",
                notification_type: "platform",
                priority: "high",
                target_role_codes: None,
            },
        )
        .await
        .expect("Failed to create platform notification");

    assert_eq!(notification.title, "Platform Notice");
    assert_eq!(notification.content, "Hello everyone");
    assert_eq!(notification.notification_type, "platform");
    assert_eq!(notification.priority, "high");
    assert_eq!(notification.status, "active");
    assert!(notification.tenant_id.is_none());
}

#[tokio::test]
#[serial]
async fn can_create_tenant_all_notification() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;
    let tenant_id = get_default_tenant_id(db).await;

    let notification =
        knota_fold::modules::notification::service::create::create_notification(
            db,
            &knota_fold::modules::notification::service::create::CreateNotificationParams {
                tenant_id: Some(tenant_id),
                created_by: user_id,
                title: "Tenant Notice",
                content: "All users",
                notification_type: "tenant_all",
                priority: "normal",
                target_role_codes: None,
            },
        )
        .await
        .expect("Failed to create tenant_all notification");

    assert_eq!(notification.title, "Tenant Notice");
    assert_eq!(notification.notification_type, "tenant_all");
    assert_eq!(notification.tenant_id, Some(tenant_id));
}

#[tokio::test]
#[serial]
async fn can_create_tenant_role_notification() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;
    let tenant_id = get_default_tenant_id(db).await;

    let role_codes = vec!["TENANT_ADMIN".to_string()];
    let notification =
        knota_fold::modules::notification::service::create::create_notification(
            db,
            &knota_fold::modules::notification::service::create::CreateNotificationParams {
                tenant_id: Some(tenant_id),
                created_by: user_id,
                title: "Role Notice",
                content: "Admins only",
                notification_type: "tenant_role",
                priority: "normal",
                target_role_codes: Some(role_codes.as_slice()),
            },
        )
        .await
        .expect("Failed to create tenant_role notification");

    assert_eq!(notification.notification_type, "tenant_role");
    assert!(notification.target_role_codes.is_some());
    let codes: Vec<String> =
        serde_json::from_str(notification.target_role_codes.as_ref().unwrap()).unwrap();
    assert_eq!(codes, vec!["TENANT_ADMIN"]);
}

#[tokio::test]
#[serial]
async fn can_revoke_notification() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;

    let notification =
        knota_fold::modules::notification::service::create::create_notification(
            db,
            &knota_fold::modules::notification::service::create::CreateNotificationParams {
                tenant_id: None,
                created_by: user_id,
                title: "To Revoke",
                content: "Will be revoked",
                notification_type: "platform",
                priority: "normal",
                target_role_codes: None,
            },
        )
        .await
        .expect("Failed to create notification");

    knota_fold::modules::notification::service::revoke::revoke_notification(
        db,
        notification.id,
        user_id,
        true,
        None,
    )
    .await
    .expect("Failed to revoke notification");
}

#[tokio::test]
#[serial]
async fn revoke_already_revoked_fails() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;

    let notification =
        knota_fold::modules::notification::service::create::create_notification(
            db,
            &knota_fold::modules::notification::service::create::CreateNotificationParams {
                tenant_id: None,
                created_by: user_id,
                title: "Double Revoke",
                content: "Should fail",
                notification_type: "platform",
                priority: "normal",
                target_role_codes: None,
            },
        )
        .await
        .expect("Failed to create notification");

    // First revoke — should succeed
    knota_fold::modules::notification::service::revoke::revoke_notification(
        db,
        notification.id,
        user_id,
        true,
        None,
    )
    .await
    .expect("First revoke should succeed");

    // Second revoke — should fail
    let result = knota_fold::modules::notification::service::revoke::revoke_notification(
        db,
        notification.id,
        user_id,
        true,
        None,
    )
    .await;

    assert!(result.is_err(), "Revoke on already-revoked should fail");
}

#[tokio::test]
#[serial]
async fn can_get_unread_count_after_notify_users() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;
    let tenant_id = get_default_tenant_id(db).await;

    // Before: count should be 0
    let before =
        knota_fold::modules::notification::service::query::get_unread_count(db, user_id)
            .await
            .expect("Failed to get unread count");
    assert_eq!(before.count, 0);

    // Send notification to self
    knota_fold::modules::notification::service::create::notify_users(
        db,
        tenant_id,
        user_id,
        "Direct Test",
        "You got a notification",
        &[user_id],
    )
    .await
    .expect("Failed to notify users");

    // After: count should be > 0
    let after =
        knota_fold::modules::notification::service::query::get_unread_count(db, user_id)
            .await
            .expect("Failed to get unread count after notification");
    assert!(
        after.count > 0,
        "Unread count should be > 0 after notification"
    );
}

#[tokio::test]
#[serial]
async fn can_mark_all_read() {
    let boot = boot_test::<App>().await.expect("Failed to boot");
    seed::<App>(&boot.app_context)
        .await
        .expect("Failed to seed");
    let db = &boot.app_context.db;

    let user_id = get_super_admin_id(db).await;
    let tenant_id = get_default_tenant_id(db).await;

    // Create a notification for the user
    knota_fold::modules::notification::service::create::notify_users(
        db,
        tenant_id,
        user_id,
        "Read Test",
        "Mark as read",
        &[user_id],
    )
    .await
    .expect("Failed to notify users");

    // Mark all read
    let count =
        knota_fold::modules::notification::service::query::mark_all_read(db, user_id)
            .await
            .expect("Failed to mark all read");
    assert!(count > 0, "Should have marked at least one as read");

    // Verify unread count is now 0
    let after =
        knota_fold::modules::notification::service::query::get_unread_count(db, user_id)
            .await
            .expect("Failed to get unread count");
    assert_eq!(
        after.count, 0,
        "Unread count should be 0 after mark all read"
    );
}
