use knota_fold::{
    app::App,
    models::_entities::{file_upload_idempotency, file_upload_parts, file_uploads},
    services::file_upload_service,
};
use loco_rs::testing::request::request;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use serial_test::serial;
use uuid::Uuid;

#[tokio::test]
#[serial]
#[ignore]
async fn purge_uploads_transitions_to_expired_then_hard_purges_after_retention() {
    request::<App, _, _>(|_request, ctx| async move {
        let now = chrono::Utc::now().fixed_offset();
        let stale_id = Uuid::new_v4();
        let hard_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();

        file_uploads::ActiveModel {
            id: Set(stale_id),
            tenant_id: Set(tenant_id),
            file_name: Set("stale.bin".to_string()),
            mime_type_hint: Set(Some("application/octet-stream".to_string())),
            expected_size: Set(5 * 1024 * 1024),
            expected_hash: Set(Some(
                "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355"
                    .to_string(),
            )),
            expected_hash_algo: Set("b3".to_string()),
            expected_hash_fast: Set(None),
            part_size: Set(5 * 1024 * 1024),
            parts_total: Set(1),
            parts_received: Set(0),
            storage_backend: Set("minio".to_string()),
            bucket: Set("knota-fold-test".to_string()),
            temp_key: Set(format!("uploads/{stale_id}/multipart.bin")),
            s3_upload_id: Set(Some("stale-s3-upload".to_string())),
            status: Set("Initiated".to_string()),
            status_reason: Set(None),
            expires_at: Set(now - chrono::Duration::hours(25)),
            expired_at: Set(None),
            completed_file_id: Set(None),
            created_by: Set(user_id),
            updated_by: Set(user_id),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        file_uploads::ActiveModel {
            id: Set(hard_id),
            tenant_id: Set(tenant_id),
            file_name: Set("hard.bin".to_string()),
            mime_type_hint: Set(Some("application/octet-stream".to_string())),
            expected_size: Set(5 * 1024 * 1024),
            expected_hash: Set(Some(
                "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355"
                    .to_string(),
            )),
            expected_hash_algo: Set("b3".to_string()),
            expected_hash_fast: Set(None),
            part_size: Set(5 * 1024 * 1024),
            parts_total: Set(1),
            parts_received: Set(1),
            storage_backend: Set("minio".to_string()),
            bucket: Set("knota-fold-test".to_string()),
            temp_key: Set(format!("uploads/{hard_id}/multipart.bin")),
            s3_upload_id: Set(None),
            status: Set("Expired".to_string()),
            status_reason: Set(Some("ttl_purged".to_string())),
            expires_at: Set(now - chrono::Duration::days(8)),
            expired_at: Set(Some(now - chrono::Duration::days(8))),
            completed_file_id: Set(None),
            created_by: Set(user_id),
            updated_by: Set(user_id),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        file_upload_idempotency::ActiveModel {
            upload_id: Set(hard_id),
            endpoint: Set("abort".to_string()),
            idempotency_key: Set("purge-case".to_string()),
            response_body: Set(br#"{"id":"dead","status":"Expired"}"#.to_vec()),
            status_code: Set(200),
            created_at: Set(now - chrono::Duration::days(8)),
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        file_upload_parts::ActiveModel {
            id: Set(Uuid::new_v4()),
            upload_id: Set(hard_id),
            part_number: Set(1),
            etag: Set("etag-hard".to_string()),
            size: Set(5 * 1024 * 1024),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        let outcome = file_upload_service::purge_uploads(&ctx).await.unwrap();
        assert_eq!(outcome.soft_deleted, 1);
        assert_eq!(outcome.hard_deleted, 1);

        let stale = file_uploads::Entity::find_by_id(stale_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stale.status, "Expired");
        assert_eq!(stale.status_reason.as_deref(), Some("ttl_purged"));
        assert!(stale.expired_at.is_some());
        assert!(stale.s3_upload_id.is_none());

        let hard = file_uploads::Entity::find_by_id(hard_id)
            .one(&ctx.db)
            .await
            .unwrap();
        assert!(hard.is_none());

        let hard_parts = file_upload_parts::Entity::find()
            .filter(file_upload_parts::Column::UploadId.eq(hard_id))
            .count(&ctx.db)
            .await
            .unwrap();
        assert_eq!(hard_parts, 0);

        let hard_idempotency = file_upload_idempotency::Entity::find()
            .filter(file_upload_idempotency::Column::UploadId.eq(hard_id))
            .count(&ctx.db)
            .await
            .unwrap();
        assert_eq!(hard_idempotency, 0);
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn purge_uploads_aborts_stale_completing_rows() {
    request::<App, _, _>(|_request, ctx| async move {
        let now = chrono::Utc::now().fixed_offset();
        let upload_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();

        file_uploads::ActiveModel {
            id: Set(upload_id),
            tenant_id: Set(tenant_id),
            file_name: Set("stale-completing.bin".to_string()),
            mime_type_hint: Set(Some("application/octet-stream".to_string())),
            expected_size: Set(5 * 1024 * 1024),
            expected_hash: Set(Some(
                "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355"
                    .to_string(),
            )),
            expected_hash_algo: Set("b3".to_string()),
            expected_hash_fast: Set(None),
            part_size: Set(5 * 1024 * 1024),
            parts_total: Set(1),
            parts_received: Set(1),
            storage_backend: Set("minio".to_string()),
            bucket: Set("knota-fold-test".to_string()),
            temp_key: Set(format!("uploads/{upload_id}/multipart.bin")),
            s3_upload_id: Set(None),
            status: Set("Completing".to_string()),
            status_reason: Set(Some("pending".to_string())),
            expires_at: Set(now + chrono::Duration::hours(1)),
            expired_at: Set(None),
            completed_file_id: Set(None),
            created_by: Set(user_id),
            updated_by: Set(user_id),
            created_at: Set(now - chrono::Duration::hours(2)),
            updated_at: Set(now - chrono::Duration::hours(2)),
        }
        .insert(&ctx.db)
        .await
        .unwrap();

        let outcome = file_upload_service::purge_uploads(&ctx).await.unwrap();
        assert_eq!(outcome.soft_deleted, 0);
        assert_eq!(outcome.hard_deleted, 0);

        let upload = file_uploads::Entity::find_by_id(upload_id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(upload.status, "Aborted");
        assert_eq!(
            upload.status_reason.as_deref(),
            Some("complete_stale_aborted")
        );
        assert!(upload.s3_upload_id.is_none());
    })
    .await;
}
