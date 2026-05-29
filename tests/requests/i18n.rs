use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

#[tokio::test]
#[serial]
async fn can_list_enabled_locales_public() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        let response = request
            .get("/api/public/i18n/locales")
            .add_header(
                prepare_data::auth_header(&admin.token).0,
                prepare_data::auth_header(&admin.token).1,
            )
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "list_enabled_locales: {response:?}"
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.is_array(), "Should return array of locales");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_bundle() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/i18n/bundles/CommonError/zh-CN")
            .add_header(auth_key.clone(), auth_value.clone())
            .await;

        assert_eq!(response.status_code(), 200, "get_bundle: {response:?}");
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.is_object(), "Bundle should be a JSON object");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn bundle_returns_etag_header() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/i18n/bundles/CommonError/zh-CN")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200);
        let etag = response
            .headers()
            .get("etag")
            .map(|v| v.to_str().unwrap().to_string());
        assert!(etag.is_some(), "Bundle response should include ETag header");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn get_bundle_requires_auth() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/i18n/bundles/CommonError/zh-CN").await;

        assert_ne!(
            response.status_code(),
            200,
            "Unauthenticated bundle request should not succeed"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admin_can_list_locales() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/admin/i18n/locales")
            .add_header(auth_key, auth_value)
            .await;

        let status = response.status_code();
        let text = response.text();
        assert_eq!(
            status, 200,
            "admin list locales — status {status} body: {text}"
        );
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(body.is_array(), "Expected array of locales, got: {text}");
        assert!(
            body.as_array().unwrap().len() >= 2,
            "Expected at least zh-CN + en-US"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admin_can_list_namespaces() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/admin/i18n/namespaces")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "admin list namespaces: {response:?}"
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.is_array(), "Expected array of namespaces");
    })
    .await;
}
