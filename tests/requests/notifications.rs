use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

// ── Unauthenticated access ────────────────────────────────────────

#[tokio::test]
#[serial]
async fn unauthenticated_list_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/notifications").await;
        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403, got {status}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn unauthenticated_create_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "Should Fail",
                "content": "No auth",
                "notificationType": "platform"
            }))
            .await;
        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403, got {status}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn unauthenticated_inbox_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/notifications/inbox").await;
        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403, got {status}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn unauthenticated_unread_count_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/notifications/unread-count").await;
        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403, got {status}"
        );
    })
    .await;
}

// ── Create notifications ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn can_create_platform_notification_as_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "Platform Notice",
                "content": "Hello everyone",
                "notificationType": "platform",
                "priority": "high"
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create platform notification should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.get("id").is_some(), "Missing 'id'");
        assert_eq!(body["title"], "Platform Notice");
        assert_eq!(body["content"], "Hello everyone");
        assert_eq!(body["notificationType"], "platform");
        assert_eq!(body["priority"], "high");
        assert_eq!(body["status"], "active");
        assert!(
            body["createdAt"].is_string(),
            "createdAt should be a string"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_tenant_all_notification_as_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "Tenant Notice",
                "content": "All tenant users",
                "notificationType": "tenant_all"
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create tenant_all notification should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["notificationType"], "tenant_all");
        assert_eq!(body["priority"], "normal");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_tenant_role_notification() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "Role Notice",
                "content": "Admins only",
                "notificationType": "tenant_role",
                "targetRoleCodes": ["TENANT_ADMIN"]
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create tenant_role notification should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["notificationType"], "tenant_role");
        assert!(body["targetRoleCodes"].is_array());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn tenant_role_without_roles_returns_400() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "Bad Request",
                "content": "No roles",
                "notificationType": "tenant_role"
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            400,
            "tenant_role without roles should return 400: {}",
            response.text()
        );
    })
    .await;
}

// ── List notifications ────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn can_list_notifications() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        // Create one first
        request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "List Test",
                "content": "Content",
                "notificationType": "platform"
            }))
            .add_header(k.clone(), v.clone())
            .await;

        // List
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let response = request.get("/api/notifications").add_header(k2, v2).await;

        assert_eq!(response.status_code(), 200, "List should succeed");
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body["items"].is_array(), "items should be an array");
        assert!(
            body["totalItems"].is_number(),
            "totalItems should be a number"
        );
        assert!(body["page"].is_number(), "page should be a number");
        assert!(body["pageSize"].is_number(), "pageSize should be a number");
        assert!(
            body["totalPages"].is_number(),
            "totalPages should be a number"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn list_with_pagination_params() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/notifications?page=1&pageSize=10")
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200);
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["page"], 1);
        assert!(body["pageSize"].as_u64().unwrap() <= 10);
    })
    .await;
}

// ── Revoke notification ───────────────────────────────────────────

#[tokio::test]
#[serial]
async fn can_revoke_notification() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        // Create
        let create_response = request
            .post("/api/notifications")
            .json(&serde_json::json!({
                "title": "To Revoke",
                "content": "Will be revoked",
                "notificationType": "platform"
            }))
            .add_header(k.clone(), v.clone())
            .await;
        let create_body: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let notification_id = create_body["id"].as_str().unwrap();

        // Revoke
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/notifications/{notification_id}/revoke"))
            .add_header(k2, v2)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Revoke should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["success"], true);
    })
    .await;
}

// ── Inbox ─────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn inbox_returns_paginated_result() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/notifications/inbox?page=1&pageSize=20")
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200, "Inbox should succeed");
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body["items"].is_array(), "items should be an array");
        assert!(body["totalItems"].is_number());
    })
    .await;
}

// ── Unread count ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn unread_count_returns_shape() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/notifications/unread-count")
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200);
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body["count"].is_number(), "count should be a number");
        assert!(
            body["hasForced"].is_boolean(),
            "hasForced should be a boolean"
        );
    })
    .await;
}

// ── Mark read / mark all read ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn can_mark_all_read() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .put("/api/notifications/read-all")
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Mark all read should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["success"], true);
        assert!(body["count"].is_number(), "count should be present");
    })
    .await;
}

// ── Forced notifications ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forced_notifications_returns_array() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/notifications/forced")
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200);
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.is_array(), "forced should return an array");
    })
    .await;
}
