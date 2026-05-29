use axum::http::Uri;
use blake3::Hash;
use knota_fold::{
    app::App,
    initializers::s3::{SharedS3Client, SharedS3Config},
    models,
    models::_entities::{
        audit_logs, file_upload_idempotency, file_upload_parts, file_uploads, files,
    },
    services::file_upload_service,
};
use loco_rs::TestServer;
use loco_rs::{app::AppContext, testing::prelude::*};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use serial_test::serial;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;

use super::prepare_data;

const PART_SIZE: usize = 5 * 1024 * 1024;
const PROBE_SIZE: i64 = 40 * 1024 * 1024;
const THRESHOLD_SIZE: i64 = 32 * 1024 * 1024;
const FAST_HASH_WINDOW: usize = 10 * 1024 * 1024;
const FAST_HASH: &str =
    "b3fast:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn probe_body(size: i64, fast_hash: &str) -> serde_json::Value {
    serde_json::json!({
        "fileName": "movie.mp4",
        "fileSize": size,
        "contentHashFast": fast_hash,
        "mimeTypeHint": "video/mp4"
    })
}

struct UploadFixture {
    auth_key: axum::http::HeaderName,
    auth_value: axum::http::HeaderValue,
}

fn payload_bytes() -> Vec<u8> {
    let mut bytes = vec![b'a'; PART_SIZE];
    bytes.extend_from_slice(b"wave2b-final-part");
    bytes
}

fn payload_hash(bytes: &[u8]) -> String {
    let hash: Hash = blake3::hash(bytes);
    format!("b3:{}", hash.to_hex())
}

fn payload_fast_hash(bytes: &[u8]) -> Option<String> {
    if bytes.len() < THRESHOLD_SIZE as usize {
        return None;
    }

    let first = &bytes[..FAST_HASH_WINDOW];
    let middle_start = (bytes.len() / 2).saturating_sub(FAST_HASH_WINDOW / 2);
    let middle = &bytes[middle_start..(middle_start + FAST_HASH_WINDOW)];
    let last = &bytes[(bytes.len() - FAST_HASH_WINDOW)..];

    let mut hasher = blake3::Hasher::new();
    hasher.update(first);
    hasher.update(middle);
    hasher.update(last);
    Some(format!("b3fast:{}", hasher.finalize().to_hex()))
}

fn large_payload_bytes() -> Vec<u8> {
    let mut bytes = vec![0_u8; PROBE_SIZE as usize];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = b'a' + (index % 23) as u8;
    }
    bytes
}

async fn upload_fixture(
    request: &TestServer,
    ctx: &AppContext,
    suffix: &str,
) -> UploadFixture {
    let tenant_admin = prepare_data::create_tenant_and_login_admin(
        request,
        &prepare_data::login_super_admin(request, ctx).await.token,
        &format!("Upload Tenant {suffix}"),
        &format!("UPLOAD_{suffix}"),
        &format!("upload-{suffix}@test.com"),
        "admin1234",
        &format!("Upload {suffix} Admin"),
    )
    .await;
    let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);
    UploadFixture {
        auth_key,
        auth_value,
    }
}

async fn upload_fixture_with_tenant(
    request: &TestServer,
    ctx: &AppContext,
    suffix: &str,
) -> (UploadFixture, String) {
    let tenant_admin = prepare_data::create_tenant_and_login_admin(
        request,
        &prepare_data::login_super_admin(request, ctx).await.token,
        &format!("Upload Tenant {suffix}"),
        &format!("UPLOAD_{suffix}"),
        &format!("upload-{suffix}@test.com"),
        "admin1234",
        &format!("Upload {suffix} Admin"),
    )
    .await;
    let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);
    (
        UploadFixture {
            auth_key,
            auth_value,
        },
        tenant_admin.tenant_id,
    )
}

async fn initiate_upload(
    request: &TestServer,
    fixture: &UploadFixture,
    idempotency_key: &str,
    bytes: &[u8],
) -> serde_json::Value {
    let response = request
        .post("/api/file-uploads")
        .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
        .add_header("idempotency-key", idempotency_key)
        .json(&serde_json::json!({
            "fileName": "video.mp4",
            "expectedSize": bytes.len(),
            "expectedHash": payload_hash(bytes),
            "expectedHashAlgo": "b3",
            "partSize": PART_SIZE,
            "mimeTypeHint": "video/mp4"
        }))
        .await;
    assert_eq!(response.status_code(), 201, "{}", response.text());
    response.json()
}

async fn sign_part(
    request: &TestServer,
    fixture: &UploadFixture,
    upload_id: &str,
    part_number: u32,
) -> serde_json::Value {
    let response = request
        .post(&format!(
            "/api/file-uploads/{upload_id}/parts/{part_number}/sign"
        ))
        .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());
    response.json()
}

async fn register_part_response(
    request: &TestServer,
    fixture: &UploadFixture,
    upload_id: &str,
    part_number: u32,
    etag: &str,
    size: usize,
    idempotency_key: &str,
) -> serde_json::Value {
    let response = request
        .post(&format!(
            "/api/file-uploads/{upload_id}/parts/{part_number}/register"
        ))
        .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
        .add_header("idempotency-key", idempotency_key)
        .json(&serde_json::json!({
            "etag": etag,
            "size": size
        }))
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());
    response.json()
}

async fn put_presigned_bytes(url: &str, bytes: &[u8]) -> (u16, String) {
    let uri: Uri = url.parse().expect("presigned url should parse");
    let host = uri.host().expect("presigned url host");
    let port = uri.port_u16().unwrap_or(80);
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    let mut stream = TcpStream::connect((host, port))
        .await
        .expect("tcp connect to minio should succeed");
    let request = format!(
        "PUT {path_and_query} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write put headers");
    stream.write_all(bytes).await.expect("write put body");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read put response");
    let response_text = String::from_utf8_lossy(&response);
    let status = response_text
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u16>().ok())
        .expect("http status code");
    let etag = response_text
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("etag:") {
                line.split_once(':')
                    .map(|(_, value)| value.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    (status, etag)
}

async fn fetch_presigned_bytes(url: &str) -> Vec<u8> {
    let uri: Uri = url.parse().expect("presigned url should parse");
    let host = uri.host().expect("presigned url host");
    let port = uri.port_u16().unwrap_or(80);
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    let mut stream = TcpStream::connect((host, port))
        .await
        .expect("tcp connect to presigned host should succeed");
    let request = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write get");

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read get");
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("http response should contain header terminator");
    response[(header_end + 4)..].to_vec()
}

async fn storage_client(ctx: &AppContext) -> (SharedS3Client, SharedS3Config) {
    let client = ctx
        .shared_store
        .get::<SharedS3Client>()
        .expect("s3 client initialized");
    let config = ctx
        .shared_store
        .get::<SharedS3Config>()
        .expect("s3 config initialized");
    (client, config)
}

async fn upload_all_parts(
    request: &TestServer,
    fixture: &UploadFixture,
    upload_id: &str,
    bytes: &[u8],
    part_size: usize,
    idempotency_prefix: &str,
) {
    for (index, part_bytes) in bytes.chunks(part_size).enumerate() {
        let part_number = index as u32 + 1;
        let sign = sign_part(request, fixture, upload_id, part_number).await;
        let (status, etag) =
            put_presigned_bytes(sign["url"].as_str().unwrap(), part_bytes).await;
        assert_eq!(status, 200);
        register_part_response(
            request,
            fixture,
            upload_id,
            part_number,
            &etag,
            part_bytes.len(),
            &format!("{idempotency_prefix}-{part_number}"),
        )
        .await;
    }
}

#[tokio::test]
#[serial]
async fn probe_miss_returns_upload_hint_and_threshold_accepts_32_mib() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "PROBE_MISS").await;

        let response = request
            .post("/api/file-uploads/probe")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .json(&probe_body(PROBE_SIZE, FAST_HASH))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["match"], "miss");
        assert_eq!(body["uploadHint"]["endpoint"], "/api/file-uploads");
        assert_eq!(body["uploadHint"]["partSize"], 5 * 1024 * 1024);
        assert_eq!(body["uploadHint"]["partsTotal"], 8);
        assert_eq!(body["uploadHint"]["concurrencyHint"], 4);
        assert_eq!(body["uploadHint"]["requiresFullHash"], true);

        let threshold = request
            .post("/api/file-uploads/probe")
            .add_header(fixture.auth_key, fixture.auth_value)
            .json(&probe_body(THRESHOLD_SIZE, FAST_HASH))
            .await;
        assert_eq!(threshold.status_code(), 200, "{}", threshold.text());
        let threshold_body: serde_json::Value = threshold.json();
        let probe_match = threshold_body["match"].as_str().unwrap();
        assert!(matches!(probe_match, "miss" | "suspect"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn probe_suspect_returns_expiry_when_fast_hash_matches_active_file() {
    request::<App, _, _>(|request, ctx| async move {
        let (fixture, tenant_id) =
            upload_fixture_with_tenant(&request, &ctx, "PROBE_SUSPECT").await;
        let tenant_id = Uuid::parse_str(&tenant_id).unwrap();
        models::files::ActiveModel {
            id: Set(Uuid::new_v4()),
            tenant_id: Set(tenant_id),
            name: Set("existing.mp4".to_string()),
            mime_type: Set("video/mp4".to_string()),
            size: Set(PROBE_SIZE),
            content_hash: Set(payload_hash(b"existing")),
            content_hash_algo: Set("b3".to_string()),
            content_hash_fast: Set(Some(FAST_HASH.to_string())),
            storage_backend: Set("minio".to_string()),
            bucket: Set("bucket".to_string()),
            storage_key: Set("files/existing.bin".to_string()),
            multipart_upload_id: Set(None),
            status: Set("ACTIVE".to_string()),
            status_reason: Set(None),
            deleted_at: Set(None),
            purge_at: Set(None),
            deleted_by: Set(None),
            uploaded_by: Set(Uuid::new_v4()),
            created_by: Set(Uuid::new_v4()),
            updated_by: Set(Uuid::new_v4()),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        let response = request
            .post("/api/file-uploads/probe")
            .add_header(fixture.auth_key, fixture.auth_value)
            .json(&probe_body(PROBE_SIZE, FAST_HASH))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["match"], "suspect");
        assert_eq!(body["requiresFullHashConfirm"], true);
        let expires_at =
            chrono::DateTime::parse_from_rfc3339(body["expiresAt"].as_str().unwrap())
                .unwrap();
        let delta = expires_at
            .signed_duration_since(chrono::Utc::now().fixed_offset())
            .num_seconds();
        assert!((240..=320).contains(&delta), "expiresAt delta was {delta}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn probe_cross_tenant_isolation_returns_miss_for_other_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_a = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Probe Tenant A",
            "PROBE_A",
            "probe-a@test.com",
            "admin1234",
            "Probe A Admin",
        )
        .await;
        let tenant_b = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Probe Tenant B",
            "PROBE_B",
            "probe-b@test.com",
            "admin1234",
            "Probe B Admin",
        )
        .await;
        let tenant_a_id = Uuid::parse_str(&tenant_a.tenant_id).unwrap();
        models::files::ActiveModel {
            id: Set(Uuid::new_v4()),
            tenant_id: Set(tenant_a_id),
            name: Set("tenant-a.mp4".to_string()),
            mime_type: Set("video/mp4".to_string()),
            size: Set(PROBE_SIZE),
            content_hash: Set(payload_hash(b"tenant-a")),
            content_hash_algo: Set("b3".to_string()),
            content_hash_fast: Set(Some(FAST_HASH.to_string())),
            storage_backend: Set("minio".to_string()),
            bucket: Set("bucket".to_string()),
            storage_key: Set("files/tenant-a.bin".to_string()),
            multipart_upload_id: Set(None),
            status: Set("ACTIVE".to_string()),
            status_reason: Set(None),
            deleted_at: Set(None),
            purge_at: Set(None),
            deleted_by: Set(None),
            uploaded_by: Set(Uuid::new_v4()),
            created_by: Set(Uuid::new_v4()),
            updated_by: Set(Uuid::new_v4()),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        let (auth_a_key, auth_a_value) = prepare_data::auth_header(&tenant_a.token);
        let (auth_b_key, auth_b_value) = prepare_data::auth_header(&tenant_b.token);

        let response_a = request
            .post("/api/file-uploads/probe")
            .add_header(auth_a_key, auth_a_value)
            .json(&probe_body(PROBE_SIZE, FAST_HASH))
            .await;
        assert_eq!(response_a.status_code(), 200, "{}", response_a.text());
        let body_a: serde_json::Value = response_a.json();
        assert_eq!(body_a["match"], "suspect");

        let response_b = request
            .post("/api/file-uploads/probe")
            .add_header(auth_b_key, auth_b_value)
            .json(&probe_body(PROBE_SIZE, FAST_HASH))
            .await;
        assert_eq!(response_b.status_code(), 200, "{}", response_b.text());
        let body_b: serde_json::Value = response_b.json();
        assert_eq!(body_b["match"], "miss");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn probe_validation_rejects_malformed_fast_hash_and_undersize() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "PROBE_VALIDATE").await;

        let malformed_probe = request
            .post("/api/file-uploads/probe")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .json(&probe_body(PROBE_SIZE, "b3fast:xyz"))
            .await;
        assert_eq!(malformed_probe.status_code(), 400, "{}", malformed_probe.text());

        let undersize = request
            .post("/api/file-uploads/probe")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .json(&probe_body(THRESHOLD_SIZE - 1, FAST_HASH))
            .await;
        assert_eq!(undersize.status_code(), 400, "{}", undersize.text());
        let undersize_body: serde_json::Value = undersize.json();
        assert_eq!(undersize_body["error"], "PROBE_BELOW_THRESHOLD");
        assert_eq!(
            undersize_body["description"],
            "Probe requires fileSize >= 32 MiB. Use /api/files (small <=5MiB) or /api/file-uploads (multipart >5MiB) directly."
        );

        let bad_initiate = request
            .post("/api/file-uploads")
            .add_header(fixture.auth_key, fixture.auth_value)
            .add_header("idempotency-key", "probe-invalid-expected-fast")
            .json(&serde_json::json!({
                "fileName": "video.mp4",
                "expectedSize": PROBE_SIZE,
                "expectedHash": payload_hash(b"bytes"),
                "expectedHashAlgo": "b3",
                "expectedHashFast": "b3fast:xyz",
                "partSize": PART_SIZE,
                "mimeTypeHint": "video/mp4"
            }))
            .await;
        assert_eq!(bad_initiate.status_code(), 400, "{}", bad_initiate.text());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn complete_upload_large_file_persists_fast_hash() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "COMPLETE_FAST_HASH_OK").await;
        let bytes = large_payload_bytes();
        let expected_fast_hash = payload_fast_hash(&bytes).unwrap();

        let init = request
            .post("/api/file-uploads")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-fast-hash-init")
            .json(&serde_json::json!({
                "fileName": "movie.mp4",
                "expectedSize": bytes.len(),
                "expectedHash": payload_hash(&bytes),
                "expectedHashAlgo": "b3",
                "expectedHashFast": expected_fast_hash,
                "partSize": PART_SIZE,
                "mimeTypeHint": "video/mp4"
            }))
            .await;
        assert_eq!(init.status_code(), 201, "{}", init.text());
        let init_body: serde_json::Value = init.json();
        let upload_id = init_body["id"].as_str().unwrap().to_string();
        let part_size = init_body["partSize"].as_u64().unwrap() as usize;

        upload_all_parts(
            &request,
            &fixture,
            &upload_id,
            &bytes,
            part_size,
            "complete-fast-hash-register",
        )
        .await;

        let complete = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-fast-hash")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(complete.status_code(), 200, "{}", complete.text());
        let body: serde_json::Value = complete.json();

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status, "Completed");

        let file = files::Entity::find_by_id(
            Uuid::parse_str(body["file"]["id"].as_str().unwrap()).unwrap(),
        )
        .one(&ctx.db)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(file.content_hash_fast, payload_fast_hash(&bytes));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn complete_upload_fast_hash_mismatch_sets_status_reason() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "COMPLETE_FAST_HASH_BAD").await;
        let bytes = large_payload_bytes();

        let init = request
            .post("/api/file-uploads")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-fast-hash-bad-init")
            .json(&serde_json::json!({
                "fileName": "movie.mp4",
                "expectedSize": bytes.len(),
                "expectedHash": payload_hash(&bytes),
                "expectedHashAlgo": "b3",
                "expectedHashFast": FAST_HASH,
                "partSize": PART_SIZE,
                "mimeTypeHint": "video/mp4"
            }))
            .await;
        assert_eq!(init.status_code(), 201, "{}", init.text());
        let init_body: serde_json::Value = init.json();
        let upload_id = init_body["id"].as_str().unwrap().to_string();
        let part_size = init_body["partSize"].as_u64().unwrap() as usize;

        upload_all_parts(
            &request,
            &fixture,
            &upload_id,
            &bytes,
            part_size,
            "complete-fast-hash-bad-register",
        )
        .await;

        let complete = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-fast-hash-bad")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(complete.status_code(), 412, "{}", complete.text());
        let body: serde_json::Value = complete.json();
        assert_eq!(body["error"], "fast_hash_mismatch");

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status_reason.as_deref(), Some("fast_hash_mismatch"));
        assert_ne!(upload.status, "Completed");
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn complete_upload_legacy_without_expected_hash_fast_persists_computed_fast_hash() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "COMPLETE_LEGACY_FAST_HASH").await;
        let bytes = large_payload_bytes();
        let expected_fast_hash = payload_fast_hash(&bytes);

        let init = request
            .post("/api/file-uploads")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-legacy-fast-hash-init")
            .json(&serde_json::json!({
                "fileName": "movie.mp4",
                "expectedSize": bytes.len(),
                "expectedHash": payload_hash(&bytes),
                "expectedHashAlgo": "b3",
                "partSize": PART_SIZE,
                "mimeTypeHint": "video/mp4"
            }))
            .await;
        assert_eq!(init.status_code(), 201, "{}", init.text());
        let init_body: serde_json::Value = init.json();
        let upload_id = init_body["id"].as_str().unwrap().to_string();
        let part_size = init_body["partSize"].as_u64().unwrap() as usize;

        upload_all_parts(
            &request,
            &fixture,
            &upload_id,
            &bytes,
            part_size,
            "complete-legacy-fast-hash-register",
        )
        .await;

        let complete = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-legacy-fast-hash")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(complete.status_code(), 200, "{}", complete.text());
        let body: serde_json::Value = complete.json();

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert!(upload.expected_hash_fast.is_none());
        assert_eq!(upload.status, "Completed");

        let file = files::Entity::find_by_id(
            Uuid::parse_str(body["file"]["id"].as_str().unwrap()).unwrap(),
        )
        .one(&ctx.db)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(file.content_hash_fast, expected_fast_hash);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn initiate_creates_initiated_session() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "INIT_CASE").await;
        let body =
            initiate_upload(&request, &fixture, "initiate-case", &payload_bytes()).await;

        assert!(body["id"].is_string());
        assert_eq!(body["status"], "Initiated");
        assert_eq!(body["partSize"], PART_SIZE as i64);
        assert_eq!(body["partsTotal"], 2);
        assert_eq!(body["presignedUrlTtlSeconds"], 3600);
        assert!(body["expiresAt"].is_string());
        assert!(body["tempKey"].as_str().unwrap().contains("uploads/"));

        let upload_id = Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();
        let upload = file_uploads::Entity::find_by_id(upload_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upload.status, "Initiated");
        assert_eq!(upload.parts_received, 0);
        assert!(upload.s3_upload_id.is_some());
        assert!(upload.completed_file_id.is_none());

        let cached = file_upload_idempotency::Entity::find_by_id((
            upload_id,
            "initiate".to_string(),
            "initiate-case".to_string(),
        ))
        .one(&ctx.db)
        .await
        .unwrap();
        assert!(cached.is_some());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sign_part_returns_presigned_upload_part_url() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "SIGN_CASE").await;
        let init =
            initiate_upload(&request, &fixture, "sign-init", &payload_bytes()).await;
        let upload_id = init["id"].as_str().unwrap();

        let body = sign_part(&request, &fixture, upload_id, 1).await;
        assert_eq!(body["uploadId"], upload_id);
        assert_eq!(body["partNumber"], 1);
        assert_eq!(body["method"], "PUT");
        assert!(body["url"].as_str().unwrap().contains("X-Amz-Signature"));
        assert_eq!(
            body["requiredHeaders"]["content-length"],
            PART_SIZE.to_string()
        );

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert!(matches!(upload.status.as_str(), "Initiated" | "InProgress"));
        let part_count = file_upload_parts::Entity::find()
            .filter(file_upload_parts::Column::UploadId.eq(upload.id))
            .count(&ctx.db)
            .await
            .unwrap();
        assert_eq!(part_count, 0);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn register_part_upserts_part_and_counts_progress() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "REGISTER_CASE").await;
        let bytes = payload_bytes();
        let init = initiate_upload(&request, &fixture, "register-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();
        let sign = sign_part(&request, &fixture, &upload_id, 1).await;

        let (status, etag) =
            put_presigned_bytes(sign["url"].as_str().unwrap(), &bytes[..PART_SIZE]).await;
        assert_eq!(status, 200);
        let body = register_part_response(
            &request,
            &fixture,
            &upload_id,
            1,
            &etag,
            PART_SIZE,
            "register-part-1",
        )
        .await;

        assert_eq!(body["uploadId"], upload_id);
        assert_eq!(body["partNumber"], 1);
        assert_eq!(body["partsReceived"], 1);
        assert_eq!(body["status"], "InProgress");

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status, "InProgress");
        assert_eq!(upload.parts_received, 1);

        let parts = file_upload_parts::Entity::find()
            .filter(file_upload_parts::Column::UploadId.eq(upload.id))
            .all(&ctx.db)
            .await
            .unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].part_number, 1);
        assert_eq!(parts[0].size, PART_SIZE as i64);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn register_part_replay_returns_exact_cached_body() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "REGISTER_REPLAY_CASE").await;
        let bytes = payload_bytes();
        let init =
            initiate_upload(&request, &fixture, "register-replay-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();

        for (part_number, part_bytes) in
            [(1_u32, &bytes[..PART_SIZE]), (2_u32, &bytes[PART_SIZE..])]
        {
            let sign = sign_part(&request, &fixture, &upload_id, part_number).await;
            let (status, etag) =
                put_presigned_bytes(sign["url"].as_str().unwrap(), part_bytes).await;
            assert_eq!(status, 200);
            register_part_response(
                &request,
                &fixture,
                &upload_id,
                part_number,
                &etag,
                part_bytes.len(),
                &format!("register-replay-{part_number}"),
            )
            .await;
        }

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/parts/1/register"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "register-replay-1")
            .json(&serde_json::json!({
                "etag": "ignored-during-replay",
                "size": PART_SIZE
            }))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["partsReceived"], 1);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn register_part_conflicting_reregister_returns_409() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "REGISTER_CONFLICT_CASE").await;
        let bytes = payload_bytes();
        let init =
            initiate_upload(&request, &fixture, "register-conflict-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();
        let sign = sign_part(&request, &fixture, &upload_id, 1).await;
        let (status, etag) =
            put_presigned_bytes(sign["url"].as_str().unwrap(), &bytes[..PART_SIZE]).await;
        assert_eq!(status, 200);

        register_part_response(
            &request,
            &fixture,
            &upload_id,
            1,
            &etag,
            PART_SIZE,
            "register-conflict-1",
        )
        .await;

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/parts/1/register"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "register-conflict-2")
            .json(&serde_json::json!({
                "etag": "different-etag",
                "size": PART_SIZE
            }))
            .await;
        assert_eq!(response.status_code(), 409, "{}", response.text());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn complete_finalizes_copies_and_creates_file_row() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "COMPLETE_CASE").await;
        let bytes = payload_bytes();
        let init = initiate_upload(&request, &fixture, "complete-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();

        for (part_number, part_bytes) in
            [(1_u32, &bytes[..PART_SIZE]), (2_u32, &bytes[PART_SIZE..])]
        {
            let sign = sign_part(&request, &fixture, &upload_id, part_number).await;
            let (status, etag) =
                put_presigned_bytes(sign["url"].as_str().unwrap(), part_bytes).await;
            assert_eq!(status, 200);
            register_part_response(
                &request,
                &fixture,
                &upload_id,
                part_number,
                &etag,
                part_bytes.len(),
                &format!("complete-register-{part_number}"),
            )
            .await;
        }

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-case")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["uploadId"], upload_id);
        assert_eq!(body["status"], "Completed");
        assert_eq!(body["file"]["size"], bytes.len() as i64);
        assert_eq!(body["file"]["contentHash"], payload_hash(&bytes));

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status, "Completed");
        assert!(upload.completed_file_id.is_some());
        assert!(upload.status_reason.is_none());

        let file = files::Entity::find_by_id(upload.completed_file_id.unwrap())
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(file.id.to_string(), body["file"]["id"].as_str().unwrap());
        assert_eq!(
            file.storage_key,
            format!(
                "files/{}/{}.bin",
                file.id,
                payload_hash(&bytes).trim_start_matches("b3:")
            )
        );

        let (s3_client, s3_config) = storage_client(&ctx).await;
        let download = s3_client
            .get_object()
            .bucket(s3_config.bucket.clone())
            .key(file.storage_key.clone())
            .presigned(
                aws_sdk_s3::presigning::PresigningConfig::expires_in(
                    std::time::Duration::from_secs(60),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let downloaded = fetch_presigned_bytes(download.uri()).await;
        assert_eq!(downloaded, bytes);

        let audit_count = audit_logs::Entity::find()
            .filter(audit_logs::Column::Action.eq("upload_complete"))
            .filter(audit_logs::Column::ResourceId.eq(upload_id.clone()))
            .count(&ctx.db)
            .await
            .unwrap();
        assert_eq!(audit_count, 1);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn abort_cleans_temp_and_marks_aborted() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "ABORT_CASE").await;
        let bytes = payload_bytes();
        let init = initiate_upload(&request, &fixture, "abort-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();
        let sign = sign_part(&request, &fixture, &upload_id, 1).await;
        let (status, etag) =
            put_presigned_bytes(sign["url"].as_str().unwrap(), &bytes[..PART_SIZE]).await;
        assert_eq!(status, 200);
        register_part_response(
            &request,
            &fixture,
            &upload_id,
            1,
            &etag,
            PART_SIZE,
            "abort-register-1",
        )
        .await;

        let response = request
            .delete(&format!("/api/file-uploads/{upload_id}"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "abort-case")
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["id"], upload_id);
        assert_eq!(body["status"], "Aborted");

        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status, "Aborted");
        assert!(upload.completed_file_id.is_none());

        let files_count = files::Entity::find().count(&ctx.db).await.unwrap();
        assert_eq!(files_count, 0);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn abort_returns_locked_when_upload_is_completing() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "ABORT_LOCKED_CASE").await;
        let bytes = payload_bytes();
        let init = initiate_upload(&request, &fixture, "abort-locked-init", &bytes).await;
        let upload_id = Uuid::parse_str(init["id"].as_str().unwrap()).unwrap();

        file_uploads::Entity::update_many()
            .col_expr(
                file_uploads::Column::Status,
                sea_orm::sea_query::Expr::value("Completing"),
            )
            .filter(file_uploads::Column::Id.eq(upload_id))
            .exec(&ctx.db)
            .await
            .unwrap();

        let response = request
            .delete(&format!("/api/file-uploads/{upload_id}"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "abort-locked")
            .await;
        assert_eq!(response.status_code(), 409, "{}", response.text());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn complete_returns_conflict_when_upload_is_completing() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "COMPLETE_BUSY_CASE").await;
        let bytes = payload_bytes();
        let init =
            initiate_upload(&request, &fixture, "complete-busy-init", &bytes).await;
        let upload_id = Uuid::parse_str(init["id"].as_str().unwrap()).unwrap();

        file_uploads::Entity::update_many()
            .col_expr(
                file_uploads::Column::Status,
                sea_orm::sea_query::Expr::value("Completing"),
            )
            .filter(file_uploads::Column::Id.eq(upload_id))
            .exec(&ctx.db)
            .await
            .unwrap();

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-busy")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(response.status_code(), 409, "{}", response.text());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn complete_on_completed_upload_replays_success_and_caches_key() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "COMPLETE_REPLAY_CASE").await;
        let bytes = payload_bytes();
        let init =
            initiate_upload(&request, &fixture, "complete-replay-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();

        for (part_number, part_bytes) in
            [(1_u32, &bytes[..PART_SIZE]), (2_u32, &bytes[PART_SIZE..])]
        {
            let sign = sign_part(&request, &fixture, &upload_id, part_number).await;
            let (status, etag) =
                put_presigned_bytes(sign["url"].as_str().unwrap(), part_bytes).await;
            assert_eq!(status, 200);
            register_part_response(
                &request,
                &fixture,
                &upload_id,
                part_number,
                &etag,
                part_bytes.len(),
                &format!("complete-replay-register-{part_number}"),
            )
            .await;
        }

        let first = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-replay-first")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(first.status_code(), 200, "{}", first.text());
        let first_body: serde_json::Value = first.json();

        let second = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-replay-second")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(second.status_code(), 200, "{}", second.text());
        let second_body: serde_json::Value = second.json();
        assert_eq!(second_body, first_body);

        let cached = file_upload_idempotency::Entity::find_by_id((
            Uuid::parse_str(&upload_id).unwrap(),
            "complete".to_string(),
            "complete-replay-second".to_string(),
        ))
        .one(&ctx.db)
        .await
        .unwrap();
        assert!(cached.is_some());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn register_part_does_not_revive_completing_upload() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "REGISTER_BUSY_CASE").await;
        let bytes = payload_bytes();
        let init =
            initiate_upload(&request, &fixture, "register-busy-init", &bytes).await;
        let upload_id = Uuid::parse_str(init["id"].as_str().unwrap()).unwrap();

        #[cfg(debug_assertions)]
        file_upload_service::set_register_part_pre_update_flip(Some(upload_id));

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/parts/1/register"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "register-busy")
            .json(&serde_json::json!({
                "etag": "etag-busy",
                "size": PART_SIZE
            }))
            .await;
        assert_eq!(response.status_code(), 409, "{}", response.text());

        let upload = file_uploads::Entity::find_by_id(upload_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upload.status, "Completing");

        let part = file_upload_parts::Entity::find()
            .filter(file_upload_parts::Column::UploadId.eq(upload_id))
            .filter(file_upload_parts::Column::PartNumber.eq(1))
            .one(&ctx.db)
            .await
            .unwrap();
        assert!(
            part.is_none(),
            "part row must roll back on guarded UPDATE conflict"
        );

        #[cfg(debug_assertions)]
        file_upload_service::set_register_part_pre_update_flip(None);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn resume_returns_uploaded_parts_or_410_for_expired() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "RESUME_CASE").await;
        let bytes = payload_bytes();
        let init = initiate_upload(&request, &fixture, "resume-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();
        let sign = sign_part(&request, &fixture, &upload_id, 1).await;
        let (status, etag) =
            put_presigned_bytes(sign["url"].as_str().unwrap(), &bytes[..PART_SIZE]).await;
        assert_eq!(status, 200);
        register_part_response(
            &request,
            &fixture,
            &upload_id,
            1,
            &etag,
            PART_SIZE,
            "resume-register-1",
        )
        .await;

        let active_resume = request
            .get(&format!("/api/file-uploads/{upload_id}"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .await;
        assert_eq!(active_resume.status_code(), 200, "{}", active_resume.text());
        let active_body: serde_json::Value = active_resume.json();
        assert_eq!(active_body["partsReceived"], 1);
        assert_eq!(active_body["uploadedParts"][0]["partNumber"], 1);

        file_uploads::Entity::update_many()
            .col_expr(
                file_uploads::Column::Status,
                sea_orm::sea_query::Expr::value("Expired"),
            )
            .col_expr(
                file_uploads::Column::StatusReason,
                sea_orm::sea_query::Expr::value("ttl_purged"),
            )
            .col_expr(
                file_uploads::Column::ExpiredAt,
                sea_orm::sea_query::Expr::value(chrono::Utc::now()),
            )
            .filter(file_uploads::Column::Id.eq(Uuid::parse_str(&upload_id).unwrap()))
            .exec(&ctx.db)
            .await
            .unwrap();

        let expired_resume = request
            .get(&format!("/api/file-uploads/{upload_id}"))
            .add_header(fixture.auth_key, fixture.auth_value)
            .await;
        assert_eq!(
            expired_resume.status_code(),
            410,
            "{}",
            expired_resume.text()
        );
        let expired_body: serde_json::Value = expired_resume.json();
        assert_eq!(expired_body["status"], "expired");
        assert_eq!(expired_body["statusReason"], "ttl_purged");
        assert!(expired_body["expiredAt"].is_string());
    })
    .await;
}

// ---------------------------------------------------------------------------
// /instant-upload: client-driven dedup confirmation (Phase C)
// ---------------------------------------------------------------------------

fn instant_body(
    file_name: &str,
    size: i64,
    full_hash: &str,
    fast_hash: &str,
) -> serde_json::Value {
    serde_json::json!({
        "fileName": file_name,
        "expectedSize": size,
        "expectedHash": full_hash,
        "expectedHashAlgo": "b3",
        "expectedHashFast": fast_hash,
        "mimeTypeHint": "video/mp4"
    })
}

#[allow(clippy::too_many_arguments)]
async fn seed_active_file(
    ctx: &AppContext,
    tenant_id: Uuid,
    name: &str,
    size: i64,
    full_hash: &str,
    fast_hash: Option<&str>,
    storage_key: &str,
) -> models::_entities::files::Model {
    models::files::ActiveModel {
        id: Set(Uuid::new_v4()),
        tenant_id: Set(tenant_id),
        name: Set(name.to_string()),
        mime_type: Set("video/mp4".to_string()),
        size: Set(size),
        content_hash: Set(full_hash.to_string()),
        content_hash_algo: Set("b3".to_string()),
        content_hash_fast: Set(fast_hash.map(str::to_string)),
        storage_backend: Set("minio".to_string()),
        bucket: Set("bucket".to_string()),
        storage_key: Set(storage_key.to_string()),
        multipart_upload_id: Set(None),
        status: Set("ACTIVE".to_string()),
        status_reason: Set(None),
        deleted_at: Set(None),
        purge_at: Set(None),
        deleted_by: Set(None),
        uploaded_by: Set(Uuid::new_v4()),
        created_by: Set(Uuid::new_v4()),
        updated_by: Set(Uuid::new_v4()),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await
    .unwrap()
}

#[tokio::test]
#[serial]
async fn instant_upload_confirms_active_file_without_revive() {
    request::<App, _, _>(|request, ctx| async move {
        let (fixture, tenant_id) =
            upload_fixture_with_tenant(&request, &ctx, "INSTANT_HIT").await;
        let tenant_id = Uuid::parse_str(&tenant_id).unwrap();
        let full_hash = payload_hash(b"instant-hit-bytes");
        let seeded = seed_active_file(
            &ctx,
            tenant_id,
            "existing.mp4",
            PROBE_SIZE,
            &full_hash,
            Some(FAST_HASH),
            "files/instant-hit.bin",
        )
        .await;

        let response = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key, fixture.auth_value)
            .add_header("idempotency-key", "instant-hit-1")
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                &full_hash,
                FAST_HASH,
            ))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["result"], "confirmed", "body={body}");
        assert_eq!(body["revived"], false, "body={body}");
        assert_eq!(body["file"]["id"], seeded.id.to_string(), "body={body}");
        assert_eq!(body["file"]["status"], "active", "body={body}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn instant_upload_revives_soft_deleted_file_within_grace() {
    request::<App, _, _>(|request, ctx| async move {
        let (fixture, tenant_id) =
            upload_fixture_with_tenant(&request, &ctx, "INSTANT_REVIVE").await;
        let tenant_id = Uuid::parse_str(&tenant_id).unwrap();
        let full_hash = payload_hash(b"instant-revive-bytes");
        let seeded = seed_active_file(
            &ctx,
            tenant_id,
            "old.mp4",
            PROBE_SIZE,
            &full_hash,
            Some(FAST_HASH),
            "files/instant-revive.bin",
        )
        .await;

        // Soft-delete with a generous grace window (purge_at far in the future).
        let now = chrono::Utc::now().fixed_offset();
        let purge_at = now + chrono::Duration::days(7);
        let mut active: models::files::ActiveModel = seeded.clone().into();
        active.status = Set("DELETED".to_string());
        active.deleted_at = Set(Some(now));
        active.purge_at = Set(Some(purge_at));
        active.deleted_by = Set(Some(Uuid::new_v4()));
        active.update(&ctx.db).await.unwrap();

        let response = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key, fixture.auth_value)
            .add_header("idempotency-key", "instant-revive-1")
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                &full_hash,
                FAST_HASH,
            ))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["result"], "confirmed");
        assert_eq!(body["revived"], true);
        assert_eq!(body["file"]["id"], seeded.id.to_string());
        assert_eq!(body["file"]["status"], "active");

        // DB-level: row is back to ACTIVE with cleared tombstone fields.
        let refreshed = files::Entity::find_by_id(seeded.id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refreshed.status, "ACTIVE");
        assert!(refreshed.deleted_at.is_none());
        assert!(refreshed.purge_at.is_none());
        assert!(refreshed.deleted_by.is_none());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn instant_upload_returns_miss_when_no_candidate_exists() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "INSTANT_MISS").await;
        let full_hash = payload_hash(b"instant-miss-bytes");

        let response = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key, fixture.auth_value)
            .add_header("idempotency-key", "instant-miss-1")
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                &full_hash,
                FAST_HASH,
            ))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["result"], "miss");
        let hint = &body["uploadHint"];
        assert_eq!(hint["endpoint"], "/api/file-uploads");
        assert!(hint["partSize"].as_u64().unwrap() > 0);
        assert!(hint["partsTotal"].as_u64().unwrap() > 0);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn instant_upload_rejects_fast_hash_mismatch_with_422() {
    request::<App, _, _>(|request, ctx| async move {
        let (fixture, tenant_id) =
            upload_fixture_with_tenant(&request, &ctx, "INSTANT_FAST_MISMATCH").await;
        let tenant_id = Uuid::parse_str(&tenant_id).unwrap();
        let full_hash = payload_hash(b"instant-fast-mismatch-bytes");
        let stored_fast =
            "b3fast:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        seed_active_file(
            &ctx,
            tenant_id,
            "stored.mp4",
            PROBE_SIZE,
            &full_hash,
            Some(stored_fast),
            "files/instant-fast-mismatch.bin",
        )
        .await;

        let response = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key, fixture.auth_value)
            .add_header("idempotency-key", "instant-fast-mismatch-1")
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                &full_hash,
                FAST_HASH,
            ))
            .await;
        assert_eq!(response.status_code(), 422, "{}", response.text());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn instant_upload_rejects_malformed_hashes() {
    request::<App, _, _>(|request, ctx| async move {
        let fixture = upload_fixture(&request, &ctx, "INSTANT_VALIDATE").await;

        let bad_full = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "instant-validate-full")
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                "b3:not-hex",
                FAST_HASH,
            ))
            .await;
        assert_eq!(bad_full.status_code(), 400, "{}", bad_full.text());

        let bad_fast = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "instant-validate-fast")
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                &payload_hash(b"x"),
                "b3fast:zzz",
            ))
            .await;
        assert_eq!(bad_fast.status_code(), 400, "{}", bad_fast.text());

        let missing_key = request
            .post("/api/file-uploads/instant-upload")
            .add_header(fixture.auth_key, fixture.auth_value)
            .json(&instant_body(
                "client.mp4",
                PROBE_SIZE,
                &payload_hash(b"x"),
                FAST_HASH,
            ))
            .await;
        assert_eq!(missing_key.status_code(), 400, "{}", missing_key.text());
    })
    .await;
}

/// Multipart `complete_upload` must revive a soft-deleted dedup winner inside
/// the grace window. Repro of the production bug: admin soft-deletes a file,
/// the user re-uploads the same content via multipart, and the
/// (tenant, hash, size) unique index forces complete_upload into its
/// `TryInsertResult::Conflicted` branch. The old code looked the winner up via
/// `find_active_by_hash` (filters deleted_at IS NULL) and returned 500
/// `dedup winner missing after conflict`, leaving the upload row stuck in
/// `completing`. Fixed by switching to `find_any_by_hash_and_size` +
/// `revive_or_use_winner`, mirroring instant_upload / small_upload.
#[tokio::test]
#[serial]
#[ignore]
async fn complete_revives_soft_deleted_winner_within_grace() {
    request::<App, _, _>(|request, ctx| async move {
        let (fixture, tenant_id) =
            upload_fixture_with_tenant(&request, &ctx, "COMPLETE_REVIVE").await;
        let tenant_id = Uuid::parse_str(&tenant_id).unwrap();
        let bytes = payload_bytes();
        let full_hash = payload_hash(&bytes);

        // Pre-seed an ACTIVE row that already owns the unique
        // (tenant, hash, size) slot so the upcoming complete_upload INSERT will
        // collide. Match the multipart payload's hash + size exactly - that's
        // the only way to trigger the conflict branch deterministically.
        let seeded = seed_active_file(
            &ctx,
            tenant_id,
            "old.mp4",
            bytes.len() as i64,
            &full_hash,
            payload_fast_hash(&bytes).as_deref(),
            "files/complete-revive-seed.bin",
        )
        .await;

        // Soft-delete with a generous grace window. revive_or_use_winner must
        // succeed and clear the tombstone fields.
        let now = chrono::Utc::now().fixed_offset();
        let purge_at = now + chrono::Duration::days(7);
        let mut active: models::files::ActiveModel = seeded.clone().into();
        active.status = Set("DELETED".to_string());
        active.deleted_at = Set(Some(now));
        active.purge_at = Set(Some(purge_at));
        active.deleted_by = Set(Some(Uuid::new_v4()));
        active.update(&ctx.db).await.unwrap();

        // Walk the full multipart flow: init -> upload parts -> complete.
        let init =
            initiate_upload(&request, &fixture, "complete-revive-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();
        for (part_number, part_bytes) in
            [(1_u32, &bytes[..PART_SIZE]), (2_u32, &bytes[PART_SIZE..])]
        {
            let sign = sign_part(&request, &fixture, &upload_id, part_number).await;
            let (status, etag) =
                put_presigned_bytes(sign["url"].as_str().unwrap(), part_bytes).await;
            assert_eq!(status, 200);
            register_part_response(
                &request,
                &fixture,
                &upload_id,
                part_number,
                &etag,
                part_bytes.len(),
                &format!("complete-revive-register-{part_number}"),
            )
            .await;
        }

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-revive-complete")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "Completed");
        assert_eq!(body["file"]["id"], seeded.id.to_string());
        assert_eq!(body["file"]["status"], "active");

        // Upload row terminalized to Completed and points at the revived
        // winner.
        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status, "Completed");
        assert_eq!(upload.completed_file_id, Some(seeded.id));

        // File row revived in place: tombstone fields cleared.
        let refreshed = files::Entity::find_by_id(seeded.id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refreshed.status, "ACTIVE");
        assert!(refreshed.deleted_at.is_none());
        assert!(refreshed.purge_at.is_none());
        assert!(refreshed.deleted_by.is_none());
    })
    .await;
}

/// Multipart `complete_upload` must reject re-upload of a soft-deleted file
/// whose grace window has expired with 410 GONE, AND it must terminalize the
/// upload row to ABORTED so a follow-up DELETE /upload doesn't 409 with
/// `upload_busy` (assert_active_upload would otherwise see it stuck in
/// `completing`).
#[tokio::test]
#[serial]
#[ignore]
async fn complete_returns_gone_and_aborts_upload_when_grace_expired() {
    request::<App, _, _>(|request, ctx| async move {
        let (fixture, tenant_id) =
            upload_fixture_with_tenant(&request, &ctx, "COMPLETE_GONE").await;
        let tenant_id = Uuid::parse_str(&tenant_id).unwrap();
        let bytes = payload_bytes();
        let full_hash = payload_hash(&bytes);

        let seeded = seed_active_file(
            &ctx,
            tenant_id,
            "old.mp4",
            bytes.len() as i64,
            &full_hash,
            payload_fast_hash(&bytes).as_deref(),
            "files/complete-gone-seed.bin",
        )
        .await;

        // Purge timestamp in the past => grace expired.
        let now = chrono::Utc::now().fixed_offset();
        let deleted_at = now - chrono::Duration::days(30);
        let purge_at = now - chrono::Duration::days(1);
        let mut active: models::files::ActiveModel = seeded.clone().into();
        active.status = Set("DELETED".to_string());
        active.deleted_at = Set(Some(deleted_at));
        active.purge_at = Set(Some(purge_at));
        active.deleted_by = Set(Some(Uuid::new_v4()));
        active.update(&ctx.db).await.unwrap();

        let init =
            initiate_upload(&request, &fixture, "complete-gone-init", &bytes).await;
        let upload_id = init["id"].as_str().unwrap().to_string();
        for (part_number, part_bytes) in
            [(1_u32, &bytes[..PART_SIZE]), (2_u32, &bytes[PART_SIZE..])]
        {
            let sign = sign_part(&request, &fixture, &upload_id, part_number).await;
            let (status, etag) =
                put_presigned_bytes(sign["url"].as_str().unwrap(), part_bytes).await;
            assert_eq!(status, 200);
            register_part_response(
                &request,
                &fixture,
                &upload_id,
                part_number,
                &etag,
                part_bytes.len(),
                &format!("complete-gone-register-{part_number}"),
            )
            .await;
        }

        let response = request
            .post(&format!("/api/file-uploads/{upload_id}/complete"))
            .add_header(fixture.auth_key.clone(), fixture.auth_value.clone())
            .add_header("idempotency-key", "complete-gone-complete")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(response.status_code(), 410, "{}", response.text());

        // Upload row aborted so DELETE /upload won't 409 upload_busy.
        let upload =
            file_uploads::Entity::find_by_id(Uuid::parse_str(&upload_id).unwrap())
                .one(&ctx.db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(upload.status, "Aborted");

        // Soft-deleted row untouched (still tombstoned, still expired).
        let refreshed = files::Entity::find_by_id(seeded.id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refreshed.status, "DELETED");
        assert!(refreshed.deleted_at.is_some());
        assert!(refreshed.purge_at.is_some());
    })
    .await;
}
