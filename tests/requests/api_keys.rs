use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

// ── Unauthenticated access ────────────────────────────────────────

#[tokio::test]
#[serial]
async fn unauthenticated_list_api_keys_returns_401() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request.get("/api/api-keys").await;
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
async fn unauthenticated_cannot_create_exchange_token() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request
            .post("/api/api-key-exchange-tokens")
            .json(&serde_json::json!({
                "name": "Should Fail",
                "roleId": "some-id"
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
async fn exchange_with_invalid_token_returns_400() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request
            .post("/api/public/api-keys/exchange")
            .json(&serde_json::json!({
                "exchangeToken": "ex_nonexistent_token_12345678"
            }))
            .await;

        assert_eq!(
            response.status_code(),
            400,
            "Invalid token should return 400: {}",
            response.text()
        );
    })
    .await;
}

// ── API Key management (super admin, Casbin bypass) ───────────────

#[tokio::test]
#[serial]
async fn can_list_api_keys_empty() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (k, v) = prepare_data::auth_header(&admin.token);

        let response = request.get("/api/api-keys").add_header(k, v).await;

        assert_eq!(response.status_code(), 200, "List API keys should succeed");
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.is_array(), "Response should be an array");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_update_api_key() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let api_key_id = create_key_via_exchange(&request, &admin.token).await;

        // Update the key
        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .put(&format!("/api/api-keys/{api_key_id}"))
            .json(&serde_json::json!({
                "name": "Updated Key Name",
                "description": "Updated description"
            }))
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200, "Update API key should succeed");
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["name"], "Updated Key Name");
        assert_eq!(body["description"], "Updated description");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_revoke_api_key() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let api_key_id = create_key_via_exchange(&request, &admin.token).await;

        // Revoke
        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .post(&format!("/api/api-keys/{api_key_id}/revoke"))
            .add_header(k, v)
            .await;

        assert_eq!(response.status_code(), 200, "Revoke should succeed");
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(
            body["revokedAt"].is_string(),
            "revokedAt should be set after revoke"
        );

        // List should still show it (soft delete)
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let list_response = request.get("/api/api-keys").add_header(k2, v2).await;
        let list_body: serde_json::Value =
            serde_json::from_str(&list_response.text()).unwrap();
        let keys = list_body.as_array().unwrap();
        assert_eq!(keys.len(), 1, "Revoked key should still appear in list");
    })
    .await;
}

// ── Exchange Token management ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn can_create_exchange_token_with_expires_at() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let role_id = get_default_tenant_admin_role_id(&request, &admin.token).await;

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .post("/api/api-key-exchange-tokens")
            .json(&serde_json::json!({
                "name": "Token With Expiry",
                "roleId": role_id,
                "description": "Testing expiresAt parsing",
                "expiresAt": "2026-12-31T23:59:59Z",
                "maxUsage": 5
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create token with expiresAt should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(
            body.get("expiresAt").is_some(),
            "expiresAt should be present"
        );
        assert_eq!(body["name"], "Token With Expiry");
        assert_eq!(body["maxUsage"], 5);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_create_exchange_token() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let role_id = get_default_tenant_admin_role_id(&request, &admin.token).await;

        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .post("/api/api-key-exchange-tokens")
            .json(&serde_json::json!({
                "name": "Test Exchange Token",
                "roleId": role_id,
                "description": "For testing",
                "maxUsage": 1
            }))
            .add_header(k, v)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create exchange token should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();

        // Verify response shape
        assert!(body.get("id").is_some(), "Missing 'id'");
        assert_eq!(body["name"], "Test Exchange Token");
        assert!(
            body["exchangeToken"].is_string(),
            "exchangeToken should be a string (plaintext)"
        );
        assert!(
            body["exchangeToken"].as_str().unwrap().starts_with("ex_"),
            "Exchange token should start with 'ex_'"
        );
        assert!(
            body["exchangeUrl"].is_string(),
            "exchangeUrl should be present"
        );
        assert!(
            body["tokenPrefix"].is_string(),
            "tokenPrefix should be present"
        );
        assert!(body.get("roleId").is_some(), "Missing 'roleId'");
        assert!(body.get("roleName").is_some(), "Missing 'roleName'");
        assert!(body.get("expiresAt").is_some(), "Missing 'expiresAt'");
        assert_eq!(body["maxUsage"], 1);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_list_exchange_tokens() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;

        // Initially empty
        let (k, v) = prepare_data::auth_header(&admin.token);
        let response = request
            .get("/api/api-key-exchange-tokens")
            .add_header(k, v)
            .await;
        assert_eq!(response.status_code(), 200);
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body.as_array().unwrap().len(), 0);

        // Create one
        create_exchange_token_raw(&request, &admin.token).await;

        // List again
        let (k2, v2) = prepare_data::auth_header(&admin.token);
        let list_response = request
            .get("/api/api-key-exchange-tokens")
            .add_header(k2, v2)
            .await;
        let list_body: serde_json::Value =
            serde_json::from_str(&list_response.text()).unwrap();
        assert_eq!(
            list_body.as_array().unwrap().len(),
            1,
            "Should have 1 exchange token after creation"
        );
    })
    .await;
}

// ── Public exchange flow (no auth) ────────────────────────────────

#[tokio::test]
#[serial]
async fn can_get_exchange_info() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (token_plaintext, _) =
            create_exchange_token_raw(&request, &admin.token).await;

        let response = request
            .get(&format!(
                "/api/public/api-keys/exchange-info?token={token_plaintext}"
            ))
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Exchange info should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body["tenantName"].is_string(), "Missing tenantName");
        assert!(body["roleName"].is_string(), "Missing roleName");
        assert!(body["expiresAt"].is_string(), "Missing expiresAt");
        assert_eq!(body["alreadyUsed"], false, "Token should not be used yet");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_exchange_key() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (token_plaintext, _) =
            create_exchange_token_raw(&request, &admin.token).await;

        // Exchange (no auth)
        let response = request
            .post("/api/public/api-keys/exchange")
            .json(&serde_json::json!({
                "exchangeToken": token_plaintext
            }))
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Exchange should succeed: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();

        assert!(body.get("apiKeyId").is_some(), "Missing apiKeyId");
        assert!(
            body["apiKey"].is_string(),
            "apiKey should be a string (plaintext)"
        );
        assert!(
            body["apiKey"].as_str().unwrap().starts_with("sk_"),
            "API key should start with 'sk_'"
        );
        assert!(body["keyPrefix"].is_string(), "keyPrefix should be present");
        assert!(body["roleName"].is_string(), "roleName should be present");
        assert!(body.get("createdAt").is_some(), "Missing createdAt");

        // Verify the key appears in the API key list
        let (k, v) = prepare_data::auth_header(&admin.token);
        let list_response = request.get("/api/api-keys").add_header(k, v).await;
        let list_body: serde_json::Value =
            serde_json::from_str(&list_response.text()).unwrap();
        let keys = list_body.as_array().unwrap();
        assert_eq!(keys.len(), 1, "Should have 1 API key after exchange");
        assert_eq!(keys[0]["id"], body["apiKeyId"]);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn exchange_token_single_use_cannot_reuse() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (token_plaintext, _) =
            create_exchange_token_raw(&request, &admin.token).await;

        // First exchange — should succeed
        let first = request
            .post("/api/public/api-keys/exchange")
            .json(&serde_json::json!({
                "exchangeToken": token_plaintext
            }))
            .await;
        assert_eq!(first.status_code(), 200, "First exchange should succeed");

        // Second exchange with same token — should fail
        let second = request
            .post("/api/public/api-keys/exchange")
            .json(&serde_json::json!({
                "exchangeToken": token_plaintext
            }))
            .await;
        assert_eq!(
            second.status_code(),
            400,
            "Reusing token should return 400: {}",
            second.text()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn exchange_info_shows_already_used_after_exchange() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (token_plaintext, _) =
            create_exchange_token_raw(&request, &admin.token).await;

        // Exchange
        request
            .post("/api/public/api-keys/exchange")
            .json(&serde_json::json!({
                "exchangeToken": token_plaintext
            }))
            .await;

        // Check exchange info again
        let info_response = request
            .get(&format!(
                "/api/public/api-keys/exchange-info?token={token_plaintext}"
            ))
            .await;
        let info_body: serde_json::Value =
            serde_json::from_str(&info_response.text()).unwrap();
        assert_eq!(
            info_body["alreadyUsed"], true,
            "Token should be marked as used after exchange"
        );
    })
    .await;
}

// ── Helpers ────────────────────────────────────────────────────────

/// Get the TENANT_ADMIN role ID for the DEFAULT tenant.
async fn get_default_tenant_admin_role_id(
    request: &loco_rs::TestServer,
    token: &str,
) -> String {
    let (k, v) = prepare_data::auth_header(token);
    let response = request
        .get("/api/roles?page=1&page_size=100&tenant_code=DEFAULT")
        .add_header(k, v)
        .await;
    assert_eq!(
        response.status_code(),
        200,
        "Get DEFAULT tenant roles failed: {}",
        response.text()
    );
    let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
    let role = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == "TENANT_ADMIN")
        .expect("TENANT_ADMIN role should exist in DEFAULT tenant");
    role["id"].as_str().unwrap().to_string()
}

/// Create an exchange token and return `(plaintext_token, token_id)`.
async fn create_exchange_token_raw(
    request: &loco_rs::TestServer,
    token: &str,
) -> (String, String) {
    let role_id = get_default_tenant_admin_role_id(request, token).await;
    let (k, v) = prepare_data::auth_header(token);
    let response = request
        .post("/api/api-key-exchange-tokens")
        .json(&serde_json::json!({
            "name": "Test Token",
            "roleId": role_id,
            "maxUsage": 1
        }))
        .add_header(k, v)
        .await;
    assert_eq!(
        response.status_code(),
        200,
        "Create exchange token helper failed: {}",
        response.text()
    );
    let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
    (
        body["exchangeToken"].as_str().unwrap().to_string(),
        body["id"].as_str().unwrap().to_string(),
    )
}

/// Create an API key via the exchange flow. Returns the API key ID.
async fn create_key_via_exchange(request: &loco_rs::TestServer, token: &str) -> String {
    let (plaintext, _token_id) = create_exchange_token_raw(request, token).await;

    let response = request
        .post("/api/public/api-keys/exchange")
        .json(&serde_json::json!({
            "exchangeToken": plaintext
        }))
        .await;
    assert_eq!(
        response.status_code(),
        200,
        "Exchange helper failed: {}",
        response.text()
    );
    let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
    body["apiKeyId"].as_str().unwrap().to_string()
}
