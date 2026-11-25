//! Real-service smoke tests for the knowledge-base QA pipeline.
//!
//! These are `#[ignore]`d because they need a live Ollama and Qdrant:
//!
//! ```bash
//! KNOWLEDGE_BASE_ENABLED=true \
//! EMBEDDING_BASE_URL=http://10.124.124.3:11434/v1 \
//! EMBEDDING_MODEL=bge-m3 \
//! QA_LLM_BASE_URL=http://10.124.124.3:11434/v1 \
//! QA_LLM_MODEL=qwen3.5:9b \
//! QDRANT_URL=http://localhost:6334 \
//! S3_ENDPOINT=http://localhost:19000 \
//! S3_ACCESS_KEY=<key> S3_SECRET_KEY=<secret> \
//! S3_TEST_BUCKET=knota-fold-test-dsh S3_FORCE_PATH_STYLE=true \
//! cargo test --test mod -- requests::qa_smoke --ignored --test-threads=1 --nocapture
//! ```

use knota_fold::app::App;
use knota_fold::config::{AppSettings, ConfigExt};
use knota_fold::modules::knowledge_base::providers::SharedEmbeddingClient;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

/// A plain question must stream tokens and finish with `Completed`.
#[tokio::test]
#[serial]
#[ignore = "requires live Ollama and Qdrant; see module docs for env vars"]
async fn qa_stream_answers_with_real_ollama() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (key, value) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/kb/qa/v3/stream")
            .json(&serde_json::json!({
                "instruction": "请直接回答：1+1 等于几？只输出数字。",
                "material": { "useKnowledgeBase": false }
            }))
            .add_header(key, value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "QA stream should start: {}",
            response.text()
        );
        let body = response.text();
        assert!(
            body.contains("\"type\":\"AnswerToken\""),
            "expected streamed answer tokens, body={body}"
        );
        assert!(
            body.contains("\"type\":\"Completed\""),
            "expected a Completed event, body={body}"
        );
        assert!(
            !body.contains("\"type\":\"Error\""),
            "unexpected stream error, body={body}"
        );
    })
    .await;
}

/// The agent must be able to call a backend tool; the SSE stream should carry
/// both `ToolCallStarted` and `ToolCallCompleted` before `Completed`.
#[tokio::test]
#[serial]
#[ignore = "requires live Ollama and Qdrant; see module docs for env vars"]
async fn qa_stream_calls_list_materials_tool() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (key, value) = prepare_data::auth_header(&admin.token);

        let response = request
            .post("/api/kb/qa/v3/stream")
            .json(&serde_json::json!({
                "instruction": "你必须先调用 list_materials 工具查看当前会话的材料列表，然后告诉我列表是否为空。不要凭空回答。",
                "material": { "useKnowledgeBase": false }
            }))
            .add_header(key, value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "QA stream should start: {}",
            response.text()
        );
        let body = response.text();
        assert!(
            body.contains("\"type\":\"ToolCallStarted\""),
            "expected ToolCallStarted, body={body}"
        );
        assert!(
            body.contains("\"type\":\"ToolCallCompleted\""),
            "expected ToolCallCompleted, body={body}"
        );
        assert!(
            body.contains("\"type\":\"Completed\""),
            "expected a Completed event, body={body}"
        );
    })
    .await;
}

/// The embedding client and both Qdrant collections must be live.
#[tokio::test]
#[serial]
#[ignore = "requires live Ollama and Qdrant; see module docs for env vars"]
async fn embedding_and_qdrant_are_reachable() {
    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let _ = admin;

        let client = ctx
            .shared_store
            .get::<SharedEmbeddingClient>()
            .expect("knowledge base initializer must be enabled");
        let settings: AppSettings = ctx
            .config
            .typed_settings()
            .expect("settings should parse")
            .expect("settings should be present");
        let kb = settings
            .knowledge_base
            .expect("knowledge base settings should be present");

        let model = client.0.embedding(kb.embedding.model.as_str(), None);
        let embedding = model
            .embed_text("knota-fold smoke test")
            .await
            .expect("embedding should succeed");
        assert_eq!(
            embedding.vec.len(),
            kb.embedding.dimension,
            "{} must return {} dimensions",
            kb.embedding.model,
            kb.embedding.dimension
        );

        let qdrant = qdrant_client::Qdrant::from_url(&kb.qdrant.url)
            .build()
            .expect("qdrant client");
        assert!(
            qdrant
                .collection_exists(&kb.qdrant.collection_name)
                .await
                .expect("kb collection query"),
            "{} collection should exist",
            kb.qdrant.collection_name
        );
        assert!(
            qdrant
                .collection_exists(&kb.qdrant.chat_collection_name)
                .await
                .expect("chat memory collection query"),
            "{} collection should exist",
            kb.qdrant.chat_collection_name
        );
    })
    .await;
}

/// Full RAG path: create a document, let the indexing worker embed it into
/// Qdrant, then ask a question that requires `search_knowledge_base`.
#[tokio::test]
#[serial]
#[ignore = "requires live Ollama and Qdrant; see module docs for env vars"]
async fn qa_stream_searches_indexed_knowledge_base() {
    use std::time::Duration;

    request::<App, _, _>(|request, ctx| async move {
        let admin = prepare_data::login_super_admin(&request, &ctx).await;
        let (key, value) = prepare_data::auth_header(&admin.token);

        let token = "KNOTA-RAG-9F3A-2026";
        let created = request
            .post("/api/kb-documents")
            .json(&serde_json::json!({
                "title": "RAG smoke document",
                "content": format!(
                    "本项目的内部代号是 {token}。这段文字只用于验证知识库检索链路。"
                ),
                "sourceType": "text/plain",
                "scope": "tenant"
            }))
            .add_header(key.clone(), value.clone())
            .await;

        assert_eq!(
            created.status_code(),
            200,
            "document create should be accepted: {}",
            created.text()
        );
        let created_body: serde_json::Value =
            serde_json::from_str(&created.text()).expect("document create json");
        let document_id = created_body["id"]
            .as_str()
            .expect("document id")
            .to_string();

        // The indexing worker runs in ForegroundBlocking mode, but poll anyway
        // so a future mode change does not make this test flaky.
        let mut ready = false;
        for _ in 0..60 {
            let detail = request
                .get(&format!("/api/kb-documents/{document_id}"))
                .add_header(key.clone(), value.clone())
                .await;
            let detail_body: serde_json::Value =
                serde_json::from_str(&detail.text()).expect("document detail json");
            if detail_body["status"] == "ready" {
                ready = true;
                break;
            }
            if detail_body["status"] == "error" {
                panic!("indexing failed: {detail_body}");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(ready, "document did not become ready in 60s");

        let response = request
            .post("/api/kb/qa/v3/stream")
            .json(&serde_json::json!({
                "instruction": "请先调用 search_knowledge_base 工具搜索“项目内部代号”，然后回答：本项目的内部代号是什么？不要直接回答。",
                "material": { "useKnowledgeBase": true }
            }))
            .add_header(key, value)
            .await;

        assert_eq!(
            response.status_code(),
            200,
            "QA stream should start: {}",
            response.text()
        );
        let body = response.text();
        assert!(
            body.contains("search_knowledge_base"),
            "expected the search_knowledge_base tool, body={body}"
        );
        assert!(
            body.contains(token),
            "answer should contain the indexed token, body={body}"
        );
        assert!(
            body.contains("\"type\":\"Completed\""),
            "expected a Completed event, body={body}"
        );
    })
    .await;
}
