use chrono::{Duration, Utc};
use knota_fold::app::App;
use knota_fold::models::_entities::api_keys;
use loco_rs::testing::prelude::*;
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait};
use serial_test::serial;

use super::prepare_data;

struct IssuedApiKey {
    id: String,
    plain_key: String,
}

#[tokio::test]
#[serial]
async fn tenant_admin_api_key_can_manage_knowledge_libraries() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "KB API integration tenant",
            "KB_API_INTEGRATION",
            "kb-api-integration@test.com",
            "admin1234",
            "KB API integration admin",
        )
        .await;
        let role_id =
            find_role_id(&request, &admin.token, "KB_API_INTEGRATION", "TENANT_ADMIN")
                .await;
        let api_key =
            issue_api_key(&request, &admin.token, &role_id, "KB integration").await;

        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let upload_probe_response = request
            .post("/api/file-uploads/probe")
            .json(&serde_json::json!({
                "fileName": "external-document.pdf",
                "fileSize": 33_554_432,
                "contentHashFast": format!("b3fast:{}", "0".repeat(64)),
                "mimeTypeHint": "application/pdf"
            }))
            .add_header(key, value)
            .await;
        assert_eq!(
            upload_probe_response.status_code(),
            200,
            "API Key should access the file upload flow: {}",
            upload_probe_response.text()
        );

        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let create_response = request
            .post("/api/kb-libraries")
            .json(&serde_json::json!({
                "name": "External API library",
                "description": "Created with an API Key"
            }))
            .add_header(key, value)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "API Key should create a library: {}",
            create_response.text()
        );

        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let list_response = request
            .get("/api/kb-libraries")
            .add_header(key, value)
            .await;
        assert_eq!(
            list_response.status_code(),
            200,
            "API Key should list libraries: {}",
            list_response.text()
        );
        let body: serde_json::Value =
            serde_json::from_str(&list_response.text()).unwrap();
        assert_eq!(body.as_array().map(Vec::len), Some(1));

        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let qa_response = request
            .post("/api/kb/qa/v3/stream")
            .json(&serde_json::json!({
                "instruction": "Use a frontend tool",
                "pageTools": [{
                    "name": "page_test",
                    "description": "Test-only frontend tool",
                    "parameters": { "type": "object" }
                }]
            }))
            .add_header(key, value)
            .await;
        assert_eq!(qa_response.status_code(), 400);
        let qa_body: serde_json::Value =
            serde_json::from_str(&qa_response.text()).unwrap();
        assert_eq!(
            qa_body["code"],
            "knowledge_base.api_key_page_tools_not_supported"
        );

        let (key, value) = prepare_data::auth_header(&admin.token);
        let jwt_response = request
            .get("/api/kb-libraries")
            .add_header(key, value)
            .await;
        assert_eq!(jwt_response.status_code(), 200, "JWT access regressed");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn api_key_without_knowledge_permission_returns_403() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "KB API denied tenant",
            "KB_API_DENIED",
            "kb-api-denied@test.com",
            "admin1234",
            "KB API denied admin",
        )
        .await;
        let member_role_id =
            find_role_id(&request, &tenant.token, "KB_API_DENIED", "MEMBER").await;
        let api_key =
            issue_api_key(&request, &tenant.token, &member_role_id, "No KB permission")
                .await;

        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let response = request
            .get("/api/kb-libraries")
            .add_header(key, value)
            .await;
        assert_eq!(
            response.status_code(),
            403,
            "API Key without permission should be rejected: {}",
            response.text()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn revoked_and_expired_api_keys_return_401() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let role_id =
            find_role_id(&request, &admin.token, "DEFAULT", "TENANT_ADMIN").await;

        let revoked =
            issue_api_key(&request, &admin.token, &role_id, "Revoked KB key").await;
        let (key, value) = prepare_data::auth_header(&admin.token);
        let revoke_response = request
            .post(&format!("/api/api-keys/{}/revoke", revoked.id))
            .add_header(key, value)
            .await;
        assert_eq!(revoke_response.status_code(), 200);

        let (key, value) = prepare_data::auth_header(&revoked.plain_key);
        let revoked_response = request
            .get("/api/kb-libraries")
            .add_header(key, value)
            .await;
        assert_eq!(revoked_response.status_code(), 401);

        let expired =
            issue_api_key(&request, &admin.token, &role_id, "Expired KB key").await;
        let expired_id = uuid::Uuid::parse_str(&expired.id).unwrap();
        let model = api_keys::Entity::find_by_id(expired_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        let mut active: api_keys::ActiveModel = model.into();
        active.expires_at =
            ActiveValue::Set(Some((Utc::now() - Duration::minutes(1)).fixed_offset()));
        active.update(&ctx.db).await.unwrap();

        let (key, value) = prepare_data::auth_header(&expired.plain_key);
        let expired_response = request
            .get("/api/kb-libraries")
            .add_header(key, value)
            .await;
        assert_eq!(expired_response.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn api_key_cannot_see_another_tenants_libraries() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_a = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "KB API tenant A",
            "KB_API_A",
            "kb-api-a@test.com",
            "admin1234",
            "KB API A admin",
        )
        .await;
        let tenant_b = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "KB API tenant B",
            "KB_API_B",
            "kb-api-b@test.com",
            "admin1234",
            "KB API B admin",
        )
        .await;

        let (key, value) = prepare_data::auth_header(&tenant_b.token);
        let create_response = request
            .post("/api/kb-libraries")
            .json(&serde_json::json!({ "name": "Tenant B private library" }))
            .add_header(key, value)
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Tenant B should create a library: {}",
            create_response.text()
        );

        let role_id =
            find_role_id(&request, &tenant_a.token, "KB_API_A", "TENANT_ADMIN").await;
        let api_key =
            issue_api_key(&request, &tenant_a.token, &role_id, "Tenant A integration")
                .await;
        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let response = request
            .get("/api/kb-libraries")
            .add_header(key, value)
            .await;
        assert_eq!(response.status_code(), 200);
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body.as_array().map(Vec::len), Some(0));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn frontend_tool_callback_rejects_api_keys() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let role_id =
            find_role_id(&request, &admin.token, "DEFAULT", "TENANT_ADMIN").await;
        let api_key =
            issue_api_key(&request, &admin.token, &role_id, "KB callback test").await;

        let (key, value) = prepare_data::auth_header(&api_key.plain_key);
        let response = request
            .post("/api/kb/qa/v3/tool-result")
            .json(&serde_json::json!({
                "toolCallId": "external-call",
                "status": "success",
                "output": {}
            }))
            .add_header(key, value)
            .await;
        assert_eq!(response.status_code(), 401);
    })
    .await;
}

async fn find_role_id(
    request: &loco_rs::TestServer,
    token: &str,
    tenant_code: &str,
    role_code: &str,
) -> String {
    let (key, value) = prepare_data::auth_header(token);
    let response = request
        .get(&format!(
            "/api/roles?page=1&pageSize=100&tenantCode={tenant_code}"
        ))
        .add_header(key, value)
        .await;
    assert_eq!(
        response.status_code(),
        200,
        "Failed to list roles for {tenant_code}: {}",
        response.text()
    );
    let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["code"] == role_code)
        .unwrap_or_else(|| panic!("Role {role_code} should exist in {tenant_code}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn issue_api_key(
    request: &loco_rs::TestServer,
    issuer_token: &str,
    role_id: &str,
    name: &str,
) -> IssuedApiKey {
    let (key, value) = prepare_data::auth_header(issuer_token);
    let token_response = request
        .post("/api/api-key-exchange-tokens")
        .json(&serde_json::json!({
            "name": name,
            "roleId": role_id,
            "maxUsage": 1
        }))
        .add_header(key, value)
        .await;
    assert_eq!(
        token_response.status_code(),
        200,
        "Failed to create exchange token: {}",
        token_response.text()
    );
    let token_body: serde_json::Value =
        serde_json::from_str(&token_response.text()).unwrap();

    let exchange_response = request
        .post("/api/public/api-keys/exchange")
        .json(&serde_json::json!({
            "exchangeToken": token_body["exchangeToken"].as_str().unwrap()
        }))
        .await;
    assert_eq!(
        exchange_response.status_code(),
        200,
        "Failed to exchange API Key: {}",
        exchange_response.text()
    );
    let body: serde_json::Value =
        serde_json::from_str(&exchange_response.text()).unwrap();

    IssuedApiKey {
        id: body["apiKeyId"].as_str().unwrap().to_string(),
        plain_key: body["apiKey"].as_str().unwrap().to_string(),
    }
}
