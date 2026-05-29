use knota_fold::{app::App, models::_entities::files, services::file_service};
use loco_rs::testing::request::request;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serial_test::serial;
use uuid::Uuid;

struct DeletedFileFixture<'a> {
    id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    bucket: &'a str,
    storage_key: String,
    content_hash: &'a str,
    size: i64,
    purge_at: chrono::DateTime<chrono::FixedOffset>,
}

fn deleted_file_active_model(fixture: DeletedFileFixture<'_>) -> files::ActiveModel {
    let deleted_at = fixture.purge_at - chrono::Duration::hours(24);
    files::ActiveModel {
        id: Set(fixture.id),
        tenant_id: Set(fixture.tenant_id),
        name: Set(format!("{}.txt", fixture.id)),
        mime_type: Set("text/plain".to_string()),
        size: Set(fixture.size),
        content_hash: Set(fixture.content_hash.to_string()),
        content_hash_algo: Set("b3".to_string()),
        content_hash_fast: Set(None),
        storage_backend: Set("minio".to_string()),
        bucket: Set(fixture.bucket.to_string()),
        storage_key: Set(fixture.storage_key),
        multipart_upload_id: Set(None),
        status: Set("DELETED".to_string()),
        status_reason: Set(Some("cleanup".to_string())),
        deleted_at: Set(Some(deleted_at)),
        purge_at: Set(Some(fixture.purge_at)),
        deleted_by: Set(Some(fixture.user_id)),
        uploaded_by: Set(fixture.user_id),
        created_by: Set(fixture.user_id),
        updated_by: Set(fixture.user_id),
        ..Default::default()
    }
}

#[tokio::test]
#[serial]
#[ignore]
async fn purge_files_hard_removes_after_grace() {
    request::<App, _, _>(|_request, ctx| async move {
        let now = chrono::Utc::now().fixed_offset();
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let expired_id = Uuid::new_v4();
        let future_id = Uuid::new_v4();

        deleted_file_active_model(DeletedFileFixture {
            id: expired_id,
            tenant_id,
            user_id,
            bucket: "knota-fold-test",
            storage_key: format!("files/{expired_id}.txt"),
            content_hash:
                "b3:dc5a4edb8240b018124052c330270696f96771a63b45250a5c17d3000e823355",
            size: 12,
            purge_at: now - chrono::Duration::minutes(5),
        })
        .insert(&ctx.db)
        .await
        .unwrap();
        deleted_file_active_model(DeletedFileFixture {
            id: future_id,
            tenant_id,
            user_id,
            bucket: "knota-fold-test",
            storage_key: format!("files/{future_id}.txt"),
            content_hash:
                "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            size: 13,
            purge_at: now + chrono::Duration::minutes(5),
        })
        .insert(&ctx.db)
        .await
        .unwrap();

        let outcome = file_service::purge_files(&ctx).await.unwrap();
        assert_eq!(outcome.purged, 1);
        assert_eq!(outcome.errors, 0);

        let expired = files::Entity::find_by_id(expired_id)
            .one(&ctx.db)
            .await
            .unwrap();
        assert!(expired.is_none(), "expired file row should be hard deleted");

        let future = files::Entity::find_by_id(future_id)
            .one(&ctx.db)
            .await
            .unwrap();
        assert!(future.is_some(), "unexpired file row should remain");
    })
    .await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn purge_files_skips_unexpired_rows() {
    request::<App, _, _>(|_request, ctx| async move {
        let now = chrono::Utc::now().fixed_offset();
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let future_id = Uuid::new_v4();

        deleted_file_active_model(DeletedFileFixture {
            id: future_id,
            tenant_id,
            user_id,
            bucket: "knota-fold-test",
            storage_key: format!("files/{future_id}.txt"),
            content_hash:
                "b3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            size: 14,
            purge_at: now + chrono::Duration::minutes(10),
        })
        .insert(&ctx.db)
        .await
        .unwrap();

        let outcome = file_service::purge_files(&ctx).await.unwrap();
        assert_eq!(outcome.purged, 0);
        assert_eq!(outcome.errors, 0);

        let future = files::Entity::find_by_id(future_id)
            .one(&ctx.db)
            .await
            .unwrap();
        assert!(future.is_some(), "unexpired file row should not be purged");
    })
    .await;
}
