use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

#[tokio::test]
#[serial]
async fn admin_can_list_worker_definitions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/worker-definitions")
            .add_header(auth_key, auth_value)
            .await;

        let status = response.status_code();
        let text = response.text();
        assert_eq!(
            status, 200,
            "list worker-definitions — status {status} body: {text}"
        );
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        let count = if body.is_object() {
            body["totalItems"].as_u64().unwrap_or(0)
        } else if body.is_array() {
            body.as_array().unwrap().len() as u64
        } else {
            0
        };
        assert!(
            count >= 1,
            "Expected at least 1 worker definition, got: {text}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admin_can_list_worker_schedules() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/worker-schedules")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200);
        let _body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admin_can_list_worker_executions() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/worker-executions")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200);
        let _body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn worker_endpoints_require_auth() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/worker-definitions").await;

        assert_ne!(
            response.status_code(),
            200,
            "Unauthenticated access should be rejected"
        );
    })
    .await;
}
