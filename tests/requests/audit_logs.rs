use knota_fold::app::App;
use knota_fold::models::_entities::audit_logs;
use loco_rs::testing::prelude::*;
use loco_rs::TestServer;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serial_test::serial;

use super::prepare_data;

async fn create_user(
    request: &TestServer,
    token: &str,
    email: &str,
    name: &str,
) -> serde_json::Value {
    let (k, v) = prepare_data::auth_header(token);
    let response = request
        .post("/api/users")
        .json(&serde_json::json!({
            "email": email,
            "password": "pass1234",
            "name": name,
        }))
        .add_header(k, v)
        .await;

    assert_eq!(
        response.status_code(),
        200,
        "Create user should succeed: {}",
        response.text()
    );

    serde_json::from_str(&response.text()).unwrap()
}

#[tokio::test]
#[serial]
async fn unauthenticated_audit_logs_returns_error() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/audit-logs").await;
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
async fn super_admin_can_query_audit_logs() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/audit-logs?page=1&page_size=10")
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Query audit logs should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body["items"].is_array(), "items should be an array");
        assert!(
            body["totalItems"].is_number(),
            "totalItems should be a number"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn create_user_generates_audit_log() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let created = create_user(
            &request,
            &admin.token,
            "audit-test-create@test.com",
            "Audit Create",
        )
        .await;
        let user_id = created["id"].as_str().expect("Created user should have id");

        let logs = audit_logs::Entity::find()
            .filter(audit_logs::Column::Action.eq("create"))
            .filter(audit_logs::Column::ResourceType.eq("user"))
            .filter(audit_logs::Column::ResourceId.eq(user_id))
            .all(&ctx.db)
            .await
            .unwrap();

        assert!(
            !logs.is_empty(),
            "Expected at least one create audit log for user {user_id}"
        );

        let log = &logs[0];
        let after_state = log
            .after_state
            .as_ref()
            .expect("Create audit log should have after_state");
        assert_eq!(after_state["email"], "audit-test-create@test.com");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn update_user_generates_audit_log() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let created = create_user(
            &request,
            &admin.token,
            "audit-test-update@test.com",
            "Audit Update Before",
        )
        .await;
        let user_id = created["id"].as_str().expect("Created user should have id");

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/users/{user_id}"))
            .json(&serde_json::json!({
                "name": "Updated Name",
            }))
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200, "Update user should succeed");

        let logs = audit_logs::Entity::find()
            .filter(audit_logs::Column::Action.eq("update"))
            .filter(audit_logs::Column::ResourceType.eq("user"))
            .filter(audit_logs::Column::ResourceId.eq(user_id))
            .all(&ctx.db)
            .await
            .unwrap();

        assert!(
            !logs.is_empty(),
            "Expected at least one update audit log for user {user_id}"
        );

        let log = &logs[0];
        assert!(
            log.before_state.is_some(),
            "Update audit log should have before_state"
        );
        assert!(
            log.after_state.is_some(),
            "Update audit log should have after_state"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn toggle_status_generates_audit_log() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let created = create_user(
            &request,
            &admin.token,
            "audit-test-status@test.com",
            "Audit Status",
        )
        .await;
        let user_id = created["id"].as_str().expect("Created user should have id");

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/users/{user_id}/status"))
            .json(&serde_json::json!({
                "status": "disabled",
            }))
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200, "Toggle status should succeed");

        let logs = audit_logs::Entity::find()
            .filter(audit_logs::Column::Action.eq("update"))
            .filter(audit_logs::Column::ResourceType.eq("user"))
            .filter(audit_logs::Column::ResourceId.eq(user_id))
            .all(&ctx.db)
            .await
            .unwrap();

        let log = logs
            .iter()
            .find(|log| {
                log.before_state
                    .as_ref()
                    .and_then(|value| value.get("status"))
                    == Some(&serde_json::Value::String("active".to_string()))
                    && log
                        .after_state
                        .as_ref()
                        .and_then(|value| value.get("status"))
                        == Some(&serde_json::Value::String("disabled".to_string()))
            })
            .expect("Expected a status change audit log");

        assert_eq!(log.before_state.as_ref().unwrap()["status"], "active");
        assert_eq!(log.after_state.as_ref().unwrap()["status"], "disabled");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn reset_password_uses_reset_password_action() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let created = create_user(
            &request,
            &admin.token,
            "audit-test-reset@test.com",
            "Audit Reset",
        )
        .await;
        let user_id = created["id"].as_str().expect("Created user should have id");

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/users/{user_id}/reset-password"))
            .json(&serde_json::json!({
                "password": "newpwd123",
            }))
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200, "Reset password should succeed");

        let logs = audit_logs::Entity::find()
            .filter(audit_logs::Column::Action.eq("reset_password"))
            .filter(audit_logs::Column::ResourceType.eq("user"))
            .filter(audit_logs::Column::ResourceId.eq(user_id))
            .all(&ctx.db)
            .await
            .unwrap();

        assert!(
            !logs.is_empty(),
            "Expected at least one reset_password audit log for user {user_id}"
        );
        assert!(logs.iter().all(|log| log.action == "reset_password"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn sync_user_roles_generates_audit_log() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        let (k, v) = prepare_data::auth_header(&admin.token);
        let roles_response = request
            .get("/api/roles?page=1&page_size=10")
            .add_header(k, v)
            .await;
        assert_eq!(
            roles_response.status_code(),
            200,
            "List roles should succeed"
        );

        let roles_body: serde_json::Value =
            serde_json::from_str(&roles_response.text()).unwrap();
        let role_id = roles_body["items"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["id"].as_str())
            .expect("Roles response should contain at least one role id");

        let created = create_user(
            &request,
            &admin.token,
            "audit-test-roles@test.com",
            "Audit Roles",
        )
        .await;
        let user_id = created["id"].as_str().expect("Created user should have id");

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/users/{user_id}/roles"))
            .json(&serde_json::json!({
                "roleIds": [role_id],
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Sync user roles should succeed: {}",
            response.text()
        );

        let logs = audit_logs::Entity::find()
            .filter(audit_logs::Column::Action.eq("update"))
            .filter(audit_logs::Column::ResourceType.eq("user_roles"))
            .filter(audit_logs::Column::ResourceId.eq(user_id))
            .all(&ctx.db)
            .await
            .unwrap();

        assert!(
            !logs.is_empty(),
            "Expected at least one user_roles audit log for user {user_id}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn filter_audit_logs_by_resource_type() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let _ = create_user(
            &request,
            &admin.token,
            "audit-test-filter-resource@test.com",
            "Audit Filter Resource",
        )
        .await;

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .get("/api/audit-logs?resource_type=user&page=1&page_size=10")
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Filter audit logs should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let items = body["items"].as_array().expect("items should be an array");

        for item in items {
            assert_eq!(
                item["resourceType"], "user",
                "All returned audit logs should have resourceType=user"
            );
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn filter_audit_logs_by_action() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let _ = create_user(
            &request,
            &admin.token,
            "audit-test-filter-action@test.com",
            "Audit Filter Action",
        )
        .await;

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .get("/api/audit-logs?action=create&page=1&page_size=10")
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Filter audit logs should succeed"
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        let items = body["items"].as_array().expect("items should be an array");

        for item in items {
            assert_eq!(
                item["action"], "create",
                "All returned audit logs should have action=create"
            );
        }
    })
    .await;
}
