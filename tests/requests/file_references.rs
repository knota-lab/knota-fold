//! Integration tests for the file_references list / attach / detach
//! HTTP surface introduced in Wave 5 D7c (tenant + sys list endpoints).
//!
//! Companion to `tests/requests/files.rs` and `tests/requests/sys_files.rs`:
//! same scaffolding, same `prepare_data` helpers, same `#[ignore]`
//! attribute (these tests boot the full app + DB + S3 and are run
//! explicitly via `cargo test -- --ignored`).
//!
//! Coverage:
//!  - tenant `GET /api/file-references`
//!      * default upload produces a `system:attachment` reference row
//!        joined with its file payload
//!      * `?resourceType=` filter narrows the list
//!      * unknown resource_type returns 400 `unknown_resource_type`
//!      * pagination metadata is wired through correctly
//!      * cross-tenant rows are NOT visible
//!  - tenant `DELETE /api/file-references/{id}` removes the row from
//!    the subsequent list (round-trip through detach + re-list)
//!  - sys `GET /api/sys/tenants/{tenantCode}/file-references`
//!      * super admin can list a target tenant's references
//!      * tenant admin (non-super) is rejected with 401

use axum_test::multipart::{MultipartForm, Part};
use knota_fold::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::prepare_data;

const HELLO_BYTES: &[u8] = b"hello world\n";

/// Upload a small file via the standard tenant endpoint. The backend's
/// `default_self_attach()` path automatically produces a
/// `system:attachment` `file_references` row whose `resource_id`
/// equals the reference id itself, so a successful upload guarantees
/// at least one reference row visible to the list endpoint.
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

/// Attach an existing file to an arbitrary business resource. Returns
/// the reference response (so callers can grab `id` for later detach).
async fn attach_reference(
    request: &loco_rs::TestServer,
    auth_key: axum::http::HeaderName,
    auth_value: axum::http::HeaderValue,
    file_id: &str,
    resource_type: &str,
    resource_id: &str,
    display_name: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "resourceType": resource_type,
        "resourceId": resource_id,
    });
    if let Some(name) = display_name {
        payload["displayName"] = serde_json::Value::String(name.to_string());
    }
    let response = request
        .post(&format!("/api/files/{file_id}/references"))
        .add_header(auth_key, auth_value)
        .json(&payload)
        .await;
    assert_eq!(
        response.status_code(),
        200,
        "attach should succeed: {}",
        response.text()
    );
    response.json()
}

#[tokio::test]
#[serial]
#[ignore]
async fn list_for_tenant_returns_default_system_attachment_with_file_payload() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Refs List Tenant",
            "REFS_LIST",
            "refs-list-admin@test.com",
            "admin1234",
            "Refs List Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload = upload_small_file(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            "list-default.txt",
            HELLO_BYTES,
        )
        .await;
        let file_id = upload["id"].as_str().unwrap();

        let list_response = request
            .get("/api/file-references")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            list_response.status_code(),
            200,
            "list should succeed: {}",
            list_response.text()
        );
        let body: serde_json::Value = list_response.json();

        // Pagination shape (PaginatedResponse<FileReferenceWithFileResponse>).
        assert!(body["items"].is_array(), "items should be an array");
        assert_eq!(body["page"].as_u64(), Some(1));
        assert!(body["pageSize"].as_u64().unwrap_or(0) > 0);
        assert!(
            body["totalItems"].as_u64().unwrap_or(0) >= 1,
            "totalItems should reflect the single seeded reference"
        );

        // Find our reference row: there should be exactly one referencing
        // the uploaded file with resourceType == system:attachment.
        let items = body["items"].as_array().unwrap();
        let row = items
            .iter()
            .find(|item| item["fileId"].as_str() == Some(file_id))
            .expect("uploaded file should produce a reference row");

        assert_eq!(row["resourceType"].as_str(), Some("system:attachment"));
        // default_self_attach contract: resource_id == reference id itself.
        assert_eq!(
            row["resourceId"].as_str(),
            row["id"].as_str(),
            "system:attachment resource_id should equal the reference id"
        );
        assert_eq!(row["fieldName"].as_str(), Some(""));

        // Joined `file` payload should be present and reference the upload.
        let file = &row["file"];
        assert!(file.is_object(), "file payload should be inlined: {row}");
        assert_eq!(file["id"].as_str(), Some(file_id));
        assert_eq!(file["name"].as_str(), Some("list-default.txt"));
        assert_eq!(file["size"].as_i64(), Some(HELLO_BYTES.len() as i64));
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn list_for_tenant_resource_type_filter_narrows_results() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Refs Filter Tenant",
            "REFS_FILTER",
            "refs-filter-admin@test.com",
            "admin1234",
            "Refs Filter Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload = upload_small_file(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            "filter.txt",
            HELLO_BYTES,
        )
        .await;
        let file_id = upload["id"].as_str().unwrap().to_string();

        // Add a second reference under a different resourceType.
        let dict_resource_id = uuid::Uuid::now_v7().to_string();
        let dict_ref = attach_reference(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            &file_id,
            "crm:contract",
            &dict_resource_id,
            Some("synonyms.txt"),
        )
        .await;
        let dict_ref_id = dict_ref["id"].as_str().unwrap().to_string();

        // Filter by crm:contract — should return only the explicit attach.
        let dict_only = request
            .get("/api/file-references?resourceType=crm:contract")
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(dict_only.status_code(), 200, "{}", dict_only.text());
        let body: serde_json::Value = dict_only.json();
        let items = body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .all(|item| item["resourceType"].as_str() == Some("crm:contract")),
            "filtered list must only contain crm:contract rows"
        );
        assert!(
            items
                .iter()
                .any(|item| item["id"].as_str() == Some(dict_ref_id.as_str())),
            "filtered list must include the attached crm:contract row"
        );

        // Filter by system:attachment — should NOT include the crm:contract row.
        let sys_only = request
            .get("/api/file-references?resourceType=system:attachment")
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(sys_only.status_code(), 200, "{}", sys_only.text());
        let body: serde_json::Value = sys_only.json();
        let items = body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .all(|item| item["resourceType"].as_str() == Some("system:attachment")),
            "filtered list must only contain system:attachment rows"
        );
        assert!(
            items
                .iter()
                .all(|item| item["id"].as_str() != Some(dict_ref_id.as_str())),
            "system:attachment filter must not include the crm:contract row"
        );

        // Unknown resourceType → 400.
        let bad = request
            .get("/api/file-references?resourceType=not:a:real:type")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(bad.status_code(), 400, "{}", bad.text());
        let bad_body: serde_json::Value = bad.json();
        assert_eq!(
            bad_body["error"].as_str(),
            Some("unknown_resource_type"),
            "unknown resourceType should map to the typed error: {bad_body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn detach_removes_reference_from_subsequent_list() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Refs Detach Tenant",
            "REFS_DETACH",
            "refs-detach-admin@test.com",
            "admin1234",
            "Refs Detach Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let upload = upload_small_file(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            "detach.txt",
            HELLO_BYTES,
        )
        .await;
        let file_id = upload["id"].as_str().unwrap().to_string();

        // Attach a second reference under crm:contract; we'll detach this one.
        let crm_resource_id = uuid::Uuid::now_v7().to_string();
        let crm_ref = attach_reference(
            &request,
            auth_key.clone(),
            auth_value.clone(),
            &file_id,
            "crm:contract",
            &crm_resource_id,
            Some("contract.txt"),
        )
        .await;
        let crm_ref_id = crm_ref["id"].as_str().unwrap().to_string();

        let detach_response = request
            .delete(&format!("/api/file-references/{crm_ref_id}"))
            .add_header(auth_key.clone(), auth_value.clone())
            .await;
        assert_eq!(
            detach_response.status_code(),
            204,
            "detach should return 204: {}",
            detach_response.text()
        );

        let after = request
            .get("/api/file-references")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(after.status_code(), 200, "{}", after.text());
        let body: serde_json::Value = after.json();
        let items = body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .all(|item| item["id"].as_str() != Some(crm_ref_id.as_str())),
            "detached reference must not appear in subsequent list"
        );
        // The default system:attachment row from upload is still present.
        assert!(
            items
                .iter()
                .any(|item| item["fileId"].as_str() == Some(file_id.as_str())
                    && item["resourceType"].as_str() == Some("system:attachment")),
            "default system:attachment row must remain after detaching the crm row"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn list_for_tenant_isolates_other_tenant_rows() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_a = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Refs Iso Tenant A",
            "REFS_ISO_A",
            "refs-iso-a@test.com",
            "admin1234",
            "Refs Iso A Admin",
        )
        .await;
        let tenant_b = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Refs Iso Tenant B",
            "REFS_ISO_B",
            "refs-iso-b@test.com",
            "admin1234",
            "Refs Iso B Admin",
        )
        .await;
        let (a_key, a_value) = prepare_data::auth_header(&tenant_a.token);
        let (b_key, b_value) = prepare_data::auth_header(&tenant_b.token);

        // Upload one file in each tenant.
        let upload_a = upload_small_file(
            &request,
            a_key.clone(),
            a_value.clone(),
            "tenant-a.txt",
            b"tenant a payload",
        )
        .await;
        let file_a = upload_a["id"].as_str().unwrap().to_string();

        let upload_b = upload_small_file(
            &request,
            b_key.clone(),
            b_value.clone(),
            "tenant-b.txt",
            b"tenant b payload",
        )
        .await;
        let file_b = upload_b["id"].as_str().unwrap().to_string();

        // Tenant A list must contain file_a, must NOT contain file_b.
        let list_a = request
            .get("/api/file-references")
            .add_header(a_key, a_value)
            .await;
        assert_eq!(list_a.status_code(), 200, "{}", list_a.text());
        let body_a: serde_json::Value = list_a.json();
        let items_a = body_a["items"].as_array().unwrap();
        assert!(
            items_a
                .iter()
                .any(|item| item["fileId"].as_str() == Some(file_a.as_str())),
            "tenant A should see its own file"
        );
        assert!(
            items_a
                .iter()
                .all(|item| item["fileId"].as_str() != Some(file_b.as_str())),
            "tenant A must NOT see tenant B's file references"
        );

        // And the symmetric check for tenant B.
        let list_b = request
            .get("/api/file-references")
            .add_header(b_key, b_value)
            .await;
        assert_eq!(list_b.status_code(), 200, "{}", list_b.text());
        let body_b: serde_json::Value = list_b.json();
        let items_b = body_b["items"].as_array().unwrap();
        assert!(
            items_b
                .iter()
                .any(|item| item["fileId"].as_str() == Some(file_b.as_str())),
            "tenant B should see its own file"
        );
        assert!(
            items_b
                .iter()
                .all(|item| item["fileId"].as_str() != Some(file_a.as_str())),
            "tenant B must NOT see tenant A's file references"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_list_for_tenant_returns_target_tenant_references() {
    request::<App, _, _>(|request, ctx| async move {
        let super_admin = prepare_data::login_super_admin(&request, &ctx).await;
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &super_admin.token,
            "Refs Sys Tenant",
            "REFS_SYS",
            "refs-sys-admin@test.com",
            "admin1234",
            "Refs Sys Admin",
        )
        .await;
        let (tenant_key, tenant_value) = prepare_data::auth_header(&tenant_admin.token);

        // Upload as the tenant admin so the file lives under that tenant.
        let upload = upload_small_file(
            &request,
            tenant_key,
            tenant_value,
            "sys-list.txt",
            HELLO_BYTES,
        )
        .await;
        let file_id = upload["id"].as_str().unwrap().to_string();

        // Super admin lists the same tenant's references via sys route.
        let (super_key, super_value) = prepare_data::auth_header(&super_admin.token);
        let response = request
            .get("/api/sys/tenants/REFS_SYS/file-references")
            .add_header(super_key, super_value)
            .await;
        assert_eq!(response.status_code(), 200, "{}", response.text());
        let body: serde_json::Value = response.json();
        let items = body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .any(|item| item["fileId"].as_str() == Some(file_id.as_str())),
            "sys list should surface the target tenant's reference"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn sys_list_for_tenant_rejects_non_super_admin() {
    request::<App, _, _>(|request, ctx| async move {
        let tenant_admin = prepare_data::create_tenant_and_login_admin(
            &request,
            &prepare_data::login_super_admin(&request, &ctx).await.token,
            "Refs Sys Forbidden Tenant",
            "REFS_SYS_FORBIDDEN",
            "refs-sys-forbidden@test.com",
            "admin1234",
            "Refs Sys Forbidden Admin",
        )
        .await;
        let (auth_key, auth_value) = prepare_data::auth_header(&tenant_admin.token);

        let response = request
            .get("/api/sys/tenants/REFS_SYS_FORBIDDEN/file-references")
            .add_header(auth_key, auth_value)
            .await;
        assert_eq!(
            response.status_code(),
            403,
            "non-super admin must be rejected by casbin (403 Forbidden, not 401): {}",
            response.text()
        );
    })
    .await;
}
