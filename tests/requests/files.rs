use axum_test::multipart::{MultipartForm, Part};
use knota_fold::{app::App, models::_entities::files};
use loco_rs::testing::prelude::*;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serial_test::serial;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;

use super::prepare_data;

const HELLO_BYTES: &[u8] = b"hello world\n";
const HELLO_HASH: &str =
    "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355";
const HELLO_SIZE: i64 = 12;

async fn fetch_presigned_bytes(url: &str) -> Vec<u8> {
    let uri: axum::http::Uri = url.parse().expect("presigned url should parse as URI");
    let host = uri.host().expect("presigned url should contain host");
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
        .expect("raw GET request should be writable");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("raw GET response should be readable");

    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("http response should contain header terminator");

    response[(header_end + 4)..].to_vec()
}

async fn upload_small_file(
    request: &loco_rs::TestServer,
    auth_key: axum::http::HeaderName,
    auth_value: axum::http::HeaderValue,
    file_name: &str,
    bytes: &[u8],
) -> serde_json::Value {
    let response = request
        .post("/api/files")
        .add_header(auth_key, auth_value)
        .multipart(
            MultipartForm::new().add_part(
                "file",
                Part::bytes(bytes.to_vec())
                    .file_name(file_name)
                    .mime_type("text/plain"),
            ),
        )
        .await;
    assert_eq!(response.status_code(), 201, "{}", response.text());
    response.json()
}

#[tokio::test]
#[serial]
#[ignore]
async fn small_upload_dedup_and_download_url_roundtrip() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Test Tenant",
            "FILES_TEST",
            "files-admin@test.com",
            "admin1234",
            "Files Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload_form = MultipartForm::new().add_part(
            "file",
            Part::bytes(HELLO_BYTES)
                .file_name("hello.txt")
                .mime_type("text/plain"),
        );

        let first_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(upload_form)
            .await;
        assert_eq!(
            first_response.status_code(),
            201,
            "first upload should create the file: {}",
            first_response.text()
        );

        let first_body: serde_json::Value = first_response.json();
        let file_id = first_body["id"]
            .as_str()
            .expect("upload response should contain id")
            .to_string();
        assert_eq!(first_body["contentHash"].as_str(), Some(HELLO_HASH));
        assert_eq!(first_body["size"].as_i64(), Some(HELLO_SIZE));

        let dedup_response = request
            .post("/api/files/dedup-check")
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({
                "contentHash": HELLO_HASH,
                "size": HELLO_SIZE,
                "name": "hello.txt"
            }))
            .await;
        assert_eq!(
            dedup_response.status_code(),
            200,
            "dedup-check should succeed: {}",
            dedup_response.text()
        );

        let dedup_body: serde_json::Value = dedup_response.json();
        assert_eq!(dedup_body["hit"].as_bool(), Some(true));
        assert_eq!(dedup_body["file"]["id"].as_str(), Some(file_id.as_str()));

        let second_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("hello-second.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            second_response.status_code(),
            201,
            "second upload should return dedup hit: {}",
            second_response.text()
        );

        let second_body: serde_json::Value = second_response.json();
        assert_eq!(second_body["id"].as_str(), Some(file_id.as_str()));
        assert_eq!(second_body["name"].as_str(), Some("hello.txt"));

        let download_response = request
            .get(&format!("/api/files/{file_id}/download-url"))
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            download_response.status_code(),
            200,
            "download-url should succeed: {}",
            download_response.text()
        );

        let download_body: serde_json::Value = download_response.json();
        let presigned_url = download_body["url"]
            .as_str()
            .expect("download-url response should include url");
        assert!(presigned_url.contains("X-Amz-Algorithm"));
        assert!(presigned_url.contains("X-Amz-Signature"));

        // Wave 2a B1 (Oracle re-review fix): TTL contract is ~1 hour.
        // Allow a small clock-skew window: expect 3500..=3600 seconds from now.
        let expires_at_str = download_body["expiresAt"]
            .as_str()
            .expect("download-url response should include expiresAt");
        let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at_str)
            .expect("expiresAt should parse as RFC3339");
        let now = chrono::Utc::now();
        let delta = expires_at.signed_duration_since(now).num_seconds();
        assert!(
            (3500..=3600).contains(&delta),
            "expiresAt should be ~1 hour in the future, got {delta}s"
        );

        let downloaded_bytes = fetch_presigned_bytes(presigned_url).await;
        assert_eq!(downloaded_bytes.as_slice(), HELLO_BYTES);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn soft_delete_then_restore_within_grace() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Soft Delete Tenant",
            "FILES_SOFT_DELETE",
            "files-soft-delete@test.com",
            "admin1234",
            "Files Soft Delete Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("soft-delete.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            upload_response.status_code(),
            201,
            "{}",
            upload_response.text()
        );
        let upload_body: serde_json::Value = upload_response.json();
        let file_id = upload_body["id"].as_str().unwrap().to_string();

        let delete_response = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "{}",
            delete_response.text()
        );

        let list_after_delete = request
            .get("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            list_after_delete.status_code(),
            200,
            "{}",
            list_after_delete.text()
        );
        let list_after_delete_body: serde_json::Value = list_after_delete.json();
        let items = list_after_delete_body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .all(|item| item["id"].as_str() != Some(file_id.as_str())),
            "soft-deleted file should be hidden from user list"
        );

        let restore_response = request
            .post(&format!("/api/files/{file_id}/restore"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(
            restore_response.status_code(),
            200,
            "{}",
            restore_response.text()
        );
        let restore_body: serde_json::Value = restore_response.json();
        assert_eq!(restore_body["id"].as_str(), Some(file_id.as_str()));
        assert!(restore_body["statusReason"].is_null());

        let list_after_restore = request
            .get("/api/files")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            list_after_restore.status_code(),
            200,
            "{}",
            list_after_restore.text()
        );
        let list_after_restore_body: serde_json::Value = list_after_restore.json();
        let items = list_after_restore_body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .any(|item| item["id"].as_str() == Some(file_id.as_str())),
            "restored file should reappear in user list"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn restore_after_grace_returns_410() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Grace Tenant",
            "FILES_GRACE",
            "files-grace@test.com",
            "admin1234",
            "Files Grace Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("grace.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            upload_response.status_code(),
            201,
            "{}",
            upload_response.text()
        );
        let upload_body: serde_json::Value = upload_response.json();
        let file_id = Uuid::parse_str(upload_body["id"].as_str().unwrap()).unwrap();

        let delete_response = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "{}",
            delete_response.text()
        );

        let file = files::Entity::find_by_id(file_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        let mut active_model: files::ActiveModel = file.into();
        active_model.purge_at = Set(Some(
            (chrono::Utc::now() - chrono::Duration::hours(1)).fixed_offset(),
        ));
        active_model.update(&ctx.db).await.unwrap();

        let restore_response = request
            .post(&format!("/api/files/{file_id}/restore"))
            .add_header(auth_key, auth_value)
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(
            restore_response.status_code(),
            410,
            "{}",
            restore_response.text()
        );
        let restore_body: serde_json::Value = restore_response.json();
        assert_eq!(restore_body["error"].as_str(), Some("grace_expired"));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn soft_delete_already_deleted_returns_409() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Already Deleted Tenant",
            "FILES_ALREADY_DELETED",
            "files-already-deleted@test.com",
            "admin1234",
            "Files Already Deleted Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("already-deleted.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            upload_response.status_code(),
            201,
            "{}",
            upload_response.text()
        );
        let upload_body: serde_json::Value = upload_response.json();
        let file_id = upload_body["id"].as_str().unwrap().to_string();

        let first_delete = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(first_delete.status_code(), 200, "{}", first_delete.text());

        let second_delete = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key, auth_value)
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(second_delete.status_code(), 409, "{}", second_delete.text());
        let second_delete_body: serde_json::Value = second_delete.json();
        assert_eq!(
            second_delete_body["error"].as_str(),
            Some("already_deleted")
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn dedup_revives_tombstone_within_grace() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Dedup Revive Tenant",
            "FILES_DEDUP_REVIVE",
            "files-dedup-revive@test.com",
            "admin1234",
            "Files Dedup Revive Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("dedup-revive.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            upload_response.status_code(),
            201,
            "{}",
            upload_response.text()
        );
        let upload_body: serde_json::Value = upload_response.json();
        let file_id = upload_body["id"].as_str().unwrap().to_string();

        let delete_response = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "{}",
            delete_response.text()
        );

        let reupload_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("dedup-revive-new.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            reupload_response.status_code(),
            201,
            "{}",
            reupload_response.text()
        );
        let reupload_body: serde_json::Value = reupload_response.json();
        assert_eq!(reupload_body["id"].as_str(), Some(file_id.as_str()));
        assert_eq!(reupload_body["statusReason"].as_str(), Some("dedup_revive"));

        let list_response = request
            .get("/api/files")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(list_response.status_code(), 200, "{}", list_response.text());
        let list_body: serde_json::Value = list_response.json();
        assert!(list_body["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"].as_str() == Some(file_id.as_str())));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn dedup_check_does_not_hit_tombstone() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Dedup Check Tenant",
            "FILES_DEDUP_CHECK",
            "files-dedup-check@test.com",
            "admin1234",
            "Files Dedup Check Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload_response = request
            .post("/api/files")
            .add_header(auth_key.clone(), auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("dedup-check.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(
            upload_response.status_code(),
            201,
            "{}",
            upload_response.text()
        );
        let upload_body: serde_json::Value = upload_response.json();
        let file_id = upload_body["id"].as_str().unwrap().to_string();

        let delete_response = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "{}",
            delete_response.text()
        );

        let dedup_response = request
            .post("/api/files/dedup-check")
            .add_header(auth_key, auth_value)
            .json(&serde_json::json!({
                "contentHash": HELLO_HASH,
                "size": HELLO_SIZE,
                "name": "dedup-check.txt"
            }))
            .await;
        assert_eq!(
            dedup_response.status_code(),
            200,
            "{}",
            dedup_response.text()
        );
        let dedup_body: serde_json::Value = dedup_response.json();
        assert_eq!(dedup_body["hit"].as_bool(), Some(false));
        assert!(dedup_body["file"].is_null());
    })
    .await;
}

// Wave 2a R2 (Oracle re-review): A full end-to-end .exe blacklist
// rejection test was attempted, but tree_magic_mini v3 (our chosen MIME
// detector) does not reliably classify synthetic PE/ELF/shell-script
// payloads on this host — it falls back to application/octet-stream or
// text/plain, which means the current blacklist of
// {x-msdownload, x-msi, x-sh, x-elf} is structurally correct but cannot
// be exercised end-to-end with a synthetic payload here.
//
// The blacklist predicate itself is covered by unit tests on the pure
// `is_blacklisted` function in `src/utils/mime.rs` (asserts both
// rejection of every blacklisted MIME and pass-through of safe MIMEs).
// A full real-binary smoke (with an actual .exe / .so file) is deferred
// to Wave 2c/2d, where we expect to either ship a richer MIME database
// with tree_magic_mini or migrate to `infer`.

#[tokio::test]
#[serial]
#[ignore]
async fn proxy_content_streams_uploaded_bytes() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Proxy Content Tenant",
            "FILES_PROXY_CONTENT",
            "files-proxy-content@test.com",
            "admin1234",
            "Files Proxy Content Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);
        let content = b"hello wave 2d proxy_content streaming test";
        let upload = upload_small_file(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            "波浪 2d.txt",
            content,
        )
        .await;
        let file_id = upload["id"].as_str().unwrap();

        let response = request
            .get(&format!("/api/files/{file_id}/content"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 200, "{}", response.text());
        assert_eq!(
            response.header("content-type").to_str().unwrap(),
            "text/plain"
        );
        let content_disposition = response.header("content-disposition");
        let content_disposition = content_disposition.to_str().unwrap();
        assert!(content_disposition.contains("filename=\""));
        assert!(content_disposition.contains("filename*=UTF-8''"));
        assert_eq!(response.as_bytes().as_ref(), content);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn proxy_content_returns_404_when_deleted() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Files Proxy Deleted Tenant",
            "FILES_PROXY_DELETED",
            "files-proxy-deleted@test.com",
            "admin1234",
            "Files Proxy Deleted Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);
        let upload = upload_small_file(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            "deleted.txt",
            b"deleted file",
        )
        .await;
        let file_id = upload["id"].as_str().unwrap().to_string();

        let delete_response = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .json(&serde_json::json!({ "reason": "cleanup" }))
            .await;
        assert_eq!(
            delete_response.status_code(),
            200,
            "{}",
            delete_response.text()
        );

        let response = request
            .get(&format!("/api/files/{file_id}/content"))
            .add_header(auth_key, auth_value)
            .await;

        assert_eq!(response.status_code(), 404, "{}", response.text());
        assert!(response.text().contains("not_found"));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn proxy_content_returns_404_when_other_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_a = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Files Proxy Tenant A",
            "FILES_PROXY_A",
            "files-proxy-a@test.com",
            "admin1234",
            "Files Proxy A Admin",
        )
        .await;
        let tenant_b = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Files Proxy Tenant B",
            "FILES_PROXY_B",
            "files-proxy-b@test.com",
            "admin1234",
            "Files Proxy B Admin",
        )
        .await;
        let (tenant_a_key, tenant_a_value) = prepare_data::auth_header(&tenant_a.token);
        let (tenant_b_key, tenant_b_value) = prepare_data::auth_header(&tenant_b.token);

        let upload = upload_small_file(
            &request,
            tenant_a_key,
            tenant_a_value,
            "cross-tenant.txt",
            b"cross tenant file",
        )
        .await;
        let file_id = upload["id"].as_str().unwrap();

        let response = request
            .get(&format!("/api/files/{file_id}/content"))
            .add_header(tenant_b_key, tenant_b_value)
            .await;

        assert_eq!(response.status_code(), 404, "{}", response.text());
        assert!(response.text().contains("not_found"));
    })
    .await;
}
