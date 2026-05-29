use axum::http::Uri;
use blake3::Hash;
use knota_fold::{
    app::App,
    models,
    models::_entities::{file_uploads, files},
};
use loco_rs::TestServer;
use loco_rs::{app::AppContext, testing::prelude::*};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serial_test::serial;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;

use super::prepare_data;

const PART_SIZE: usize = 5 * 1024 * 1024;
const PROBE_SIZE: i64 = 40 * 1024 * 1024;
const FAST_HASH: &str =
    "b3fast:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn payload_bytes() -> Vec<u8> {
    let mut bytes = vec![b'b'; PART_SIZE];
    bytes.extend_from_slice(b"sys-wave2b-final-part");
    bytes
}

fn payload_hash(bytes: &[u8]) -> String {
    let hash: Hash = blake3::hash(bytes);
    format!("b3:{}", hash.to_hex())
}

fn probe_body() -> serde_json::Value {
    serde_json::json!({
        "fileName": "movie.mp4",
        "fileSize": PROBE_SIZE,
        "contentHashFast": FAST_HASH,
        "mimeTypeHint": "video/mp4"
    })
}

async fn create_super_admin_target(
    request: &TestServer,
    ctx: &AppContext,
    suffix: &str,
) -> (String, String) {
    let super_admin = prepare_data::login_super_admin(request, ctx).await;
    let tenant_admin = prepare_data::create_tenant_and_login_admin(
        request,
        &super_admin.token,
        &format!("Sys Upload Tenant {suffix}"),
        &format!("SYS_UPLOAD_{suffix}"),
        &format!("sys-upload-{suffix}@test.com"),
        "admin1234",
        &format!("Sys Upload {suffix} Admin"),
    )
    .await;
    (super_admin.token, tenant_admin.tenant_id)
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
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(bytes).await.unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response_text = String::from_utf8_lossy(&response);
    let status = response_text
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap();
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

#[tokio::test]
#[serial]
async fn probe_sys_route_uses_tenant_uuid_path() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_id) =
            create_super_admin_target(&request, &ctx, "PROBE_CASE").await;
        let tenant_uuid = Uuid::parse_str(&tenant_id).unwrap();
        models::files::ActiveModel {
            id: Set(Uuid::new_v4()),
            tenant_id: Set(tenant_uuid),
            name: Set("sys-existing.mp4".to_string()),
            mime_type: Set("video/mp4".to_string()),
            size: Set(PROBE_SIZE),
            content_hash: Set(payload_hash(b"sys-existing")),
            content_hash_algo: Set("b3".to_string()),
            content_hash_fast: Set(Some(FAST_HASH.to_string())),
            storage_backend: Set("minio".to_string()),
            bucket: Set("bucket".to_string()),
            storage_key: Set("files/sys-existing.bin".to_string()),
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

        let (auth_key, auth_value) = prepare_data::auth_header(&super_token);
        let response = request
            .post(&format!("/api/sys/tenants/{tenant_id}/file-uploads/probe"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&probe_body())
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["match"], "suspect");

        let wrong_route = request
            .post("/api/sys/tenants/SYS_UPLOAD_PROBE_CASE/file-uploads/probe")
            .add_header(auth_key, auth_value)
            .json(&probe_body())
            .await;
        assert!(wrong_route.status_code().is_client_error());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_initiate_creates_cross_tenant_upload() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_id) =
            create_super_admin_target(&request, &ctx, "INIT_CASE").await;
        let (auth_key, auth_value) = prepare_data::auth_header(&super_token);
        let bytes = payload_bytes();

        let response = request
            .post(&format!("/api/sys/tenants/{tenant_id}/file-uploads"))
            .add_header(auth_key, auth_value)
            .add_header("idempotency-key", "sys-initiate-case")
            .json(&serde_json::json!({
                "fileName": "video.mp4",
                "expectedSize": bytes.len(),
                "expectedHash": payload_hash(&bytes),
                "expectedHashAlgo": "b3",
                "partSize": PART_SIZE,
                "mimeTypeHint": "video/mp4"
            }))
            .await;

        assert_eq!(response.status_code(), 201, "{}", response.text());
        let body: serde_json::Value = response.json();
        let upload = file_uploads::Entity::find_by_id(
            Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
        )
        .one(&ctx.db)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(upload.tenant_id.to_string(), tenant_id);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_complete_happy_path_creates_file_for_target_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_id) = create_super_admin_target(&request, &ctx, "COMPLETE_CASE").await;
        let (auth_key, auth_value) = prepare_data::auth_header(&super_token);
        let bytes = payload_bytes();

        let init = request
            .post(&format!("/api/sys/tenants/{tenant_id}/file-uploads"))
            .add_header(auth_key.clone(), auth_value.clone())
            .add_header("idempotency-key", "sys-complete-init")
            .json(&serde_json::json!({
                "fileName": "video.mp4",
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

        for (part_number, part_bytes) in [(1_u32, &bytes[..PART_SIZE]), (2_u32, &bytes[PART_SIZE..])] {
            let sign = request
                .post(&format!(
                    "/api/sys/tenants/{tenant_id}/file-uploads/{upload_id}/parts/{part_number}/sign"
                ))
                .add_header(auth_key.clone(), auth_value.clone())
                .json(&serde_json::json!({}))
                .await;
            assert_eq!(sign.status_code(), 200, "{}", sign.text());
            let sign_body: serde_json::Value = sign.json();
            let (status, etag) = put_presigned_bytes(sign_body["url"].as_str().unwrap(), part_bytes).await;
            assert_eq!(status, 200);

            let register = request
                .post(&format!(
                    "/api/sys/tenants/{tenant_id}/file-uploads/{upload_id}/parts/{part_number}/register"
                ))
                .add_header(auth_key.clone(), auth_value.clone())
                .add_header("idempotency-key", &format!("sys-register-{part_number}"))
                .json(&serde_json::json!({
                    "etag": etag,
                    "size": part_bytes.len()
                }))
                .await;
            assert_eq!(register.status_code(), 200, "{}", register.text());
        }

        let complete = request
            .post(&format!(
                "/api/sys/tenants/{tenant_id}/file-uploads/{upload_id}/complete"
            ))
            .add_header(auth_key, auth_value)
            .add_header("idempotency-key", "sys-complete")
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(complete.status_code(), 200, "{}", complete.text());
        let body: serde_json::Value = complete.json();

        let file = files::Entity::find_by_id(Uuid::parse_str(body["file"]["id"].as_str().unwrap()).unwrap())
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(file.tenant_id.to_string(), tenant_id);
        assert_eq!(file.content_hash, payload_hash(&bytes));
    })
    .await;
}
