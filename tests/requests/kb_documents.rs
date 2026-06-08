use knota_fold::app::App;
use knota_fold::models::_entities::{kb_documents, kb_libraries};
use loco_rs::testing::prelude::*;
use sea_orm::{ActiveModelTrait, ActiveValue};
use serial_test::serial;
use uuid::Uuid;

use super::prepare_data;

#[tokio::test]
#[serial]
async fn unauthenticated_create_returns_error() {
    request::<App, _, _>(|request, _ctx| async move {
        let response = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "Unauthorized Doc",
                "content": "should not be created",
                "sourceType": "text/plain"
            }))
            .await;

        let status = response.status_code();
        assert!(
            status == 401 || status == 403,
            "Expected 401 or 403 without auth, got {status}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_create_document() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "Test KB Document",
                "content": "This is test content for the knowledge base document.",
                "sourceType": "text/plain"
            }))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Create document should succeed: {}",
            response.text()
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.get("id").is_some(), "Response should have 'id'");
        assert_eq!(body["title"].as_str(), Some("Test KB Document"));
        assert!(
            body.get("status").is_some(),
            "Response should have 'status'"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_list_documents() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        let response = request
            .get("/api/kb-documents?page=1&pageSize=20")
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "List documents should succeed: {}",
            response.text()
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(body.get("items").is_some(), "Response should have 'items'");
        assert!(
            body.get("totalPages").is_some(),
            "Response should have 'totalPages'"
        );
        assert!(
            body.get("totalItems").is_some(),
            "Response should have 'totalItems'"
        );
        assert!(body.get("page").is_some(), "Response should have 'page'");
        assert!(
            body.get("pageSize").is_some(),
            "Response should have 'pageSize'"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn document_list_rejects_cross_tenant_library_scope() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_b = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "KB Isolation B",
            "KB_ISOL_B",
            "kb-isol-b@test.com",
            "admin1234",
            "KB Isolation B Admin",
        )
        .await;

        let tenant_b_id = Uuid::parse_str(&tenant_b.tenant_id).unwrap();
        let user_id = Uuid::nil();
        let library_b_id = Uuid::now_v7();
        insert_library(
            &ctx.db,
            tenant_b_id,
            library_b_id,
            "tenant-b-library",
            user_id,
        )
        .await;
        insert_document(
            &ctx.db,
            tenant_b_id,
            Uuid::now_v7(),
            library_b_id,
            "tenant-b-document",
            user_id,
        )
        .await;

        let (key, value) = prepare_data::auth_header(&super_admin.token);
        let response = request
            .get(&format!(
                "/api/kb-documents?page=1&pageSize=20&libraryId={library_b_id}"
            ))
            .add_header(key, value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Cross-tenant library list should not fail: {}",
            response.text()
        );
        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["totalItems"].as_i64(), Some(0));
        assert_eq!(body["items"].as_array().map(Vec::len), Some(0));
    })
    .await;
}

async fn insert_library(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    library_id: Uuid,
    name: &str,
    user_id: Uuid,
) {
    kb_libraries::ActiveModel {
        id: ActiveValue::Set(library_id),
        tenant_id: ActiveValue::Set(tenant_id),
        name: ActiveValue::Set(name.to_string()),
        description: ActiveValue::Set(None),
        sort_order: ActiveValue::Set(0),
        created_by: ActiveValue::Set(user_id),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

async fn insert_document(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    document_id: Uuid,
    library_id: Uuid,
    title: &str,
    user_id: Uuid,
) {
    kb_documents::ActiveModel {
        id: ActiveValue::Set(document_id),
        tenant_id: ActiveValue::Set(tenant_id),
        title: ActiveValue::Set(title.to_string()),
        description: ActiveValue::Set(None),
        library_id: ActiveValue::Set(Some(library_id)),
        folder_id: ActiveValue::Set(None),
        source_type: ActiveValue::Set("text/plain".to_string()),
        file_id: ActiveValue::Set(None),
        file_reference_id: ActiveValue::Set(None),
        full_text: ActiveValue::Set(Some("cross tenant content".to_string())),
        status: ActiveValue::Set("ready".to_string()),
        scope: ActiveValue::Set("tenant".to_string()),
        chunk_count: ActiveValue::Set(1),
        total_tokens: ActiveValue::Set(3),
        metadata: ActiveValue::Set(None),
        error_message: ActiveValue::Set(None),
        created_by: ActiveValue::Set(user_id),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_get_document() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        // Create a document first
        let create_response = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "Get Test Document",
                "content": "Content for get test.",
                "sourceType": "text/plain"
            }))
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create document should succeed: {}",
            create_response.text()
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let doc_id = created["id"].as_str().expect("Created doc should have id");

        // Get the document
        let response = request
            .get(&format!("/api/kb-documents/{doc_id}"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Get document should succeed: {}",
            response.text()
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(body["id"].as_str(), Some(doc_id));
        assert_eq!(body["title"].as_str(), Some("Get Test Document"));
        assert!(
            body.get("status").is_some(),
            "Response should have 'status'"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_delete_document() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        // Create a document first
        let create_response = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "Delete Test Document",
                "content": "Content for delete test.",
                "sourceType": "text/plain"
            }))
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create document should succeed: {}",
            create_response.text()
        );

        let created: serde_json::Value =
            serde_json::from_str(&create_response.text()).unwrap();
        let doc_id = created["id"].as_str().expect("Created doc should have id");

        // Delete the document
        let delete_response = request
            .delete(&format!("/api/kb-documents/{doc_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "Delete document should succeed: {}",
            delete_response.text()
        );

        let delete_body: serde_json::Value =
            serde_json::from_str(&delete_response.text()).unwrap();
        assert_eq!(delete_body["success"].as_bool(), Some(true));

        // Verify it's gone — GET should return an error
        let get_response = request
            .get(&format!("/api/kb-documents/{doc_id}"))
            .add_header(auth_key, auth_value)
            .await;
        assert_ne!(
            get_response.status_code(),
            200,
            "Deleted document should not be found"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_search() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        // Create a document with known content so there's something to search
        let create_response = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "Search Test Document",
                "content": "The quick brown fox jumps over the lazy dog. This is unique searchable text for testing knowledge base search.",
                "sourceType": "text/plain"
            }))
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create document for search should succeed: {}",
            create_response.text()
        );

        // Search
        let response = request
            .post("/api/kb/search")
            .json(&serde_json::json!({
                "query": "quick brown fox",
                "limit": 5
            }))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Search should succeed: {}",
            response.text()
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(
            body.is_array(),
            "Search response should be an array: {body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_qa() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/kb/qa/v3/stream")
            .json(&serde_json::json!({
                "instruction": "Summarize the following text",
                "material": {
                    "inline": "Knowledge bases store information in structured ways. They enable semantic search and question answering over documents."
                }
            }))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "QA should succeed: {}",
            response.text()
        );

        let body = response.text();
        assert!(
            body.contains("\"type\":\"Completed\""),
            "QA SSE should include a Completed event: {body}"
        );
        assert!(
            body.contains("\"answer\""),
            "QA SSE should include an answer payload: {body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn can_get_chunks() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = super::prepare_data::login_super_admin(&request, &ctx).await;
        let (auth_key, auth_value) = super::prepare_data::auth_header(&admin.token);

        // Create a document first
        let create_response = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "Chunks Test Document",
                "content": "This is content that will be chunked. It needs to be long enough to produce at least one chunk for the knowledge base system.",
                "sourceType": "text/plain"
            }))
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            create_response.status_code(),
            200,
            "Create document for chunks should succeed: {}",
            create_response.text()
        );

        let created: serde_json::Value = serde_json::from_str(&create_response.text()).unwrap();
        let doc_id = created["id"].as_str().expect("Created doc should have id");

        // Get chunks for the document
        let response = request
            .get(&format!("/api/kb/documents/{doc_id}/chunks"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "Get chunks should succeed: {}",
            response.text()
        );

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(
            body.is_array(),
            "Chunks response should be an array: {body}"
        );
    })
    .await;
}
