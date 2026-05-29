use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

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
            .post("/api/kb/qa")
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

        let body: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(
            body.get("answer").is_some(),
            "QA response should have 'answer'"
        );
        assert!(
            !body["answer"].as_str().unwrap_or("").is_empty(),
            "Answer should not be empty"
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
