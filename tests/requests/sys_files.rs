use axum_test::multipart::{MultipartForm, Part};
use knota_fold::{
    app::App,
    models::_entities::{audit_logs, files},
};
use loco_rs::testing::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serial_test::serial;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;

use super::prepare_data;

const HELLO_BYTES: &[u8] = b"hello world\n";

async fn create_super_admin_and_target(
    request: &loco_rs::TestServer,
    ctx: &loco_rs::app::AppContext,
    suffix: &str,
) -> (String, String, String) {
    let super_admin = prepare_data::login_super_admin(request, ctx).await;
    let tenant_code = format!("SYS_FILES_{suffix}");
    let tenant_admin = prepare_data::create_tenant_and_login_admin(
        request,
        &super_admin.token,
        &format!("Sys Files Tenant {suffix}"),
        &tenant_code,
        &format!("sys-files-{suffix}@test.com"),
        "admin1234",
        &format!("Sys Files {suffix} Admin"),
    )
    .await;
    (super_admin.token, tenant_code, tenant_admin.token)
}

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
async fn sys_can_see_soft_deleted() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_code, tenant_admin_token) =
            create_super_admin_and_target(&request, &ctx, "SEE").await;
        let (super_key, super_value) = prepare_data::auth_header(&super_token);
        let (user_key, user_value) = prepare_data::auth_header(&tenant_admin_token);

        let upload = request
            .post("/api/files")
            .add_header(user_key.clone(), user_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("sys-see.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(upload.status_code(), 201, "{}", upload.text());
        let upload_body: serde_json::Value = upload.json();
        let file_id = upload_body["id"].as_str().unwrap().to_string();

        let soft_delete = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(user_key, user_value)
            .json(&serde_json::json!({ "reason": "ops" }))
            .await;
        assert_eq!(soft_delete.status_code(), 200, "{}", soft_delete.text());

        let sys_list = request
            .get(&format!("/api/sys/tenants/{tenant_code}/files"))
            .add_header(super_key.clone(), super_value.clone())
            .await;
        assert_eq!(sys_list.status_code(), 200, "{}", sys_list.text());
        let sys_list_body: serde_json::Value = sys_list.json();
        assert!(sys_list_body["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"].as_str() == Some(file_id.as_str())));

        let sys_get = request
            .get(&format!("/api/sys/tenants/{tenant_code}/files/{file_id}"))
            .add_header(super_key, super_value)
            .await;
        assert_eq!(sys_get.status_code(), 200, "{}", sys_get.text());
        let sys_get_body: serde_json::Value = sys_get.json();
        assert_eq!(sys_get_body["id"].as_str(), Some(file_id.as_str()));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_can_download_soft_deleted() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_code, tenant_admin_token) =
            create_super_admin_and_target(&request, &ctx, "DOWNLOAD").await;
        let (super_key, super_value) = prepare_data::auth_header(&super_token);
        let (user_key, user_value) = prepare_data::auth_header(&tenant_admin_token);

        let upload = request
            .post("/api/files")
            .add_header(user_key.clone(), user_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("sys-download.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(upload.status_code(), 201, "{}", upload.text());
        let upload_body: serde_json::Value = upload.json();
        let file_id = upload_body["id"].as_str().unwrap().to_string();

        let soft_delete = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(user_key, user_value)
            .json(&serde_json::json!({ "reason": "ops" }))
            .await;
        assert_eq!(soft_delete.status_code(), 200, "{}", soft_delete.text());

        let sys_download = request
            .get(&format!(
                "/api/sys/tenants/{tenant_code}/files/{file_id}/download-url"
            ))
            .add_header(super_key, super_value)
            .await;
        assert_eq!(sys_download.status_code(), 200, "{}", sys_download.text());
        let body: serde_json::Value = sys_download.json();
        let url = body["url"].as_str().unwrap();
        let bytes = fetch_presigned_bytes(url).await;
        assert_eq!(bytes.as_slice(), HELLO_BYTES);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_can_restore_within_grace() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_code, tenant_admin_token) =
            create_super_admin_and_target(&request, &ctx, "RESTORE").await;
        let (super_key, super_value) = prepare_data::auth_header(&super_token);
        let (user_key, user_value) = prepare_data::auth_header(&tenant_admin_token);

        let upload = request
            .post("/api/files")
            .add_header(user_key.clone(), user_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("sys-restore.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(upload.status_code(), 201, "{}", upload.text());
        let upload_body: serde_json::Value = upload.json();
        let file_id = Uuid::parse_str(upload_body["id"].as_str().unwrap()).unwrap();

        let soft_delete = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(user_key, user_value)
            .json(&serde_json::json!({ "reason": "ops" }))
            .await;
        assert_eq!(soft_delete.status_code(), 200, "{}", soft_delete.text());

        let restore = request
            .post(&format!(
                "/api/sys/tenants/{tenant_code}/files/{file_id}/restore"
            ))
            .add_header(super_key.clone(), super_value.clone())
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(restore.status_code(), 200, "{}", restore.text());

        let restored = files::Entity::find_by_id(file_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert!(restored.deleted_at.is_none());

        let sys_delete = request
            .delete(&format!("/api/sys/tenants/{tenant_code}/files/{file_id}"))
            .add_header(super_key.clone(), super_value.clone())
            .json(&serde_json::json!({ "reason": "ops" }))
            .await;
        assert_eq!(sys_delete.status_code(), 200, "{}", sys_delete.text());

        let sys_restore = request
            .post(&format!(
                "/api/sys/tenants/{tenant_code}/files/{file_id}/restore"
            ))
            .add_header(super_key, super_value)
            .json(&serde_json::json!({}))
            .await;
        assert_eq!(sys_restore.status_code(), 200, "{}", sys_restore.text());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_small_upload_into_other_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let target = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Sys Files Upload Tenant",
            "SYS_FILES_UPLOAD",
            "sys-files-upload@test.com",
            "admin1234",
            "Sys Files Upload Admin",
        )
        .await;
        let (super_key, super_value) = prepare_data::auth_header(&super_admin.token);

        let response = request
            .post("/api/sys/tenants/SYS_FILES_UPLOAD/files")
            .add_header(super_key, super_value)
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("sys-small-upload.txt")
                        .mime_type("text/plain"),
                ),
            )
            .await;
        assert_eq!(response.status_code(), 201, "{}", response.text());
        let body: serde_json::Value = response.json();
        assert_eq!(body["tenantId"].as_str(), Some(target.tenant_id.as_str()));
        assert_eq!(
            body["uploadedBy"].as_str(),
            Some(super_admin.user.id.to_string().as_str())
        );

        let audit = audit_logs::Entity::find()
            .filter(audit_logs::Column::ResourceType.eq("file"))
            .filter(audit_logs::Column::ResourceId.eq(body["id"].as_str().unwrap()))
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.tenant_id.to_string(), target.tenant_id);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_dedup_check_scoped_to_target_tenant() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_a = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Sys Dedup Tenant A",
            "SYS_DEDUP_A",
            "sys-dedup-a@test.com",
            "admin1234",
            "Sys Dedup A Admin",
        )
        .await;
        let _tenant_b = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Sys Dedup Tenant B",
            "SYS_DEDUP_B",
            "sys-dedup-b@test.com",
            "admin1234",
            "Sys Dedup B Admin",
        )
        .await;
        let (super_key, super_value) = prepare_data::auth_header(&super_admin.token);
        let (tenant_a_key, tenant_a_value) = prepare_data::auth_header(&tenant_a.token);

        upload_small_file(
            &request,
            tenant_a_key,
            tenant_a_value,
            "hello.txt",
            HELLO_BYTES,
        )
        .await;

        let tenant_b_response = request
            .post("/api/sys/tenants/SYS_DEDUP_B/files/dedup-check")
            .add_header(super_key.clone(), super_value.clone())
            .json(&serde_json::json!({
                "contentHash": "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355",
                "size": 12,
                "name": "hello.txt"
            }))
            .await;
        assert_eq!(tenant_b_response.status_code(), 200, "{}", tenant_b_response.text());
        let tenant_b_body: serde_json::Value = tenant_b_response.json();
        assert_eq!(tenant_b_body["hit"].as_bool(), Some(false));

        let tenant_a_response = request
            .post("/api/sys/tenants/SYS_DEDUP_A/files/dedup-check")
            .add_header(super_key, super_value)
            .json(&serde_json::json!({
                "contentHash": "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355",
                "size": 12,
                "name": "hello.txt"
            }))
            .await;
        assert_eq!(tenant_a_response.status_code(), 200, "{}", tenant_a_response.text());
        let tenant_a_body: serde_json::Value = tenant_a_response.json();
        assert_eq!(tenant_a_body["hit"].as_bool(), Some(true));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_proxy_content_streams_deleted_file() {
    request::<App, _, _>(|request, ctx| async move {
        let (super_token, tenant_code, tenant_admin_token) =
            create_super_admin_and_target(&request, &ctx, "PROXY").await;
        let (super_key, super_value) = prepare_data::auth_header(&super_token);
        let (user_key, user_value) = prepare_data::auth_header(&tenant_admin_token);

        let upload = upload_small_file(
            &request,
            user_key.clone(),
            user_value.clone(),
            "sys-proxy.txt",
            HELLO_BYTES,
        )
        .await;
        let file_id = upload["id"].as_str().unwrap().to_string();

        let soft_delete = request
            .delete(&format!("/api/files/{file_id}"))
            .add_header(user_key, user_value)
            .json(&serde_json::json!({ "reason": "ops" }))
            .await;
        assert_eq!(soft_delete.status_code(), 200, "{}", soft_delete.text());

        let response = request
            .get(&format!(
                "/api/sys/tenants/{tenant_code}/files/{file_id}/content"
            ))
            .add_header(super_key, super_value)
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        assert_eq!(response.as_bytes().as_ref(), HELLO_BYTES);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_endpoints_reject_non_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Sys Guard Tenant",
            "SYS_GUARD",
            "sys-guard@test.com",
            "admin1234",
            "Sys Guard Admin",
        )
        .await;
        let (user_key, user_value) = prepare_data::auth_header(&tenant_admin.token);

        let response = request
            .post("/api/sys/tenants/SYS_GUARD/files")
            .add_header(user_key, user_value)
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(HELLO_BYTES)
                        .file_name("forbidden.txt")
                        .mime_type("text/plain"),
                ),
            )
            .expect_failure()
            .await;

        assert_eq!(response.status_code(), 403, "{}", response.text());
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_small_upload_accepts_near_max_size() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let _target = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Sys Max Size Tenant",
            "SYS_MAX_SIZE",
            "sys-max-size@test.com",
            "admin1234",
            "Sys Max Size Admin",
        )
        .await;
        let (super_key, super_value) = prepare_data::auth_header(&super_admin.token);
        let bytes = vec![b'x'; 4 * 1024 * 1024];

        let response = request
            .post("/api/sys/tenants/SYS_MAX_SIZE/files")
            .add_header(super_key, super_value)
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(bytes)
                        .file_name("near-max.bin")
                        .mime_type("application/octet-stream"),
                ),
            )
            .await;

        assert_eq!(response.status_code(), 201, "{}", response.text());
    })
    .await;
}
