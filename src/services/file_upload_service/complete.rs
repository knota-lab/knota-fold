use super::{
    audit_service, best_effort_terminalize_upload_row, cache_success,
    cleanup_complete_failure, complete_multipart_upload, completed_response, db_err_into,
    delete_object_if_exists, delete_temp_prefix, detect_mime, ensure_idempotency_key,
    ensure_parts_continuity, fetch_completed_file, file_reference_service, file_repo,
    file_service, file_upload_parts, file_uploads, final_storage_key,
    finalize_inserted_file, find_cached_success_by_upload, insert_file_row,
    is_blacklisted, json_endpoint_response, load_parts, load_upload,
    parse_completed_file_id, require_shared_s3_client, stream_object_hashes, AuditAction,
    CompleteUploadRequest, FileAuditSnapshot, FileUploadAuditSnapshot,
    JsonEndpointResponse, SharedS3Client, COMPLETE_COPY_FAILED_REASON,
    COMPLETE_DB_FAILED_REASON, COMPLETE_OBJECT_READ_FAILED_REASON,
    COMPLETE_RETRYABLE_STATUS_REASON, ENDPOINT_COMPLETE,
    FAST_HASH_MISMATCH_STATUS_REASON, FAST_HASH_THRESHOLD, HASH_MISMATCH_STATUS_REASON,
    MIME_BLACKLIST_STATUS_REASON, SIZE_MISMATCH_STATUS_REASON, STATUS_ABORTED,
    STATUS_COMPLETED, STATUS_COMPLETING, STATUS_EXPIRED, STATUS_INITIATED,
    STATUS_IN_PROGRESS,
};
use crate::utils::error::IntoAppError;
use axum::http::StatusCode;
use chrono::Utc;
use loco_rs::controller::ErrorDetail;
use loco_rs::prelude::Error;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, TransactionTrait,
    TryInsertResult,
};
use uuid::Uuid;

pub(super) async fn complete_upload_inner(
    req: CompleteUploadRequest<'_>,
) -> loco_rs::Result<JsonEndpointResponse> {
    let key = ensure_idempotency_key(req.idempotency_key)?;
    if let Some(cached) = find_cached_success_by_upload(
        &req.ctx.db,
        req.tenant_id,
        req.upload_id,
        ENDPOINT_COMPLETE,
        key,
    )
    .await?
    {
        return Ok(cached);
    }

    let upload = load_upload(&req.ctx.db, req.tenant_id, req.upload_id).await?;
    match upload.status.as_str() {
        STATUS_COMPLETED => return complete_upload_cached(&req, &upload, key).await,
        STATUS_COMPLETING => {
            return Err(crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "upload_busy",
                "upload cannot be completed while completing or completed",
            ));
        }
        STATUS_ABORTED => {
            return Err(crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "upload_aborted",
                "upload has been aborted",
            ));
        }
        STATUS_EXPIRED => {
            return Err(crate::views::errors::err_custom(
                StatusCode::GONE,
                "upload_expired",
                "upload has expired",
            ));
        }
        STATUS_INITIATED | STATUS_IN_PROGRESS => {}
        _ => {
            return Err(crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "upload_invalid_state",
                "upload cannot be completed in its current state",
            ));
        }
    }

    let parts = load_parts(&req.ctx.db, req.upload_id).await?;
    ensure_parts_continuity(&upload, &parts)?;

    let lock_result = file_uploads::Entity::update_many()
        .col_expr(file_uploads::Column::Status, Expr::value(STATUS_COMPLETING))
        .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
        .col_expr(file_uploads::Column::UpdatedBy, Expr::value(req.user_id))
        .filter(file_uploads::Column::Id.eq(req.upload_id))
        .filter(file_uploads::Column::TenantId.eq(req.tenant_id))
        .filter(
            file_uploads::Column::Status
                .is_in([STATUS_INITIATED.to_string(), STATUS_IN_PROGRESS.to_string()]),
        )
        .exec(&req.ctx.db)
        .await
        .db_err()?;

    if lock_result.rows_affected == 0 {
        return complete_upload_locked_out(&req, &upload, key).await;
    }

    let s3_client = require_shared_s3_client(req.ctx)?;
    let s3_upload_id = upload.s3_upload_id.clone().ok_or_else(|| {
        crate::views::errors::err_custom(
            StatusCode::GONE,
            "upload_expired",
            "multipart upload is no longer active",
        )
    })?;

    complete_upload_after_lock(req, upload, parts, &s3_client, s3_upload_id, key).await
}

async fn complete_upload_cached(
    req: &CompleteUploadRequest<'_>,
    upload: &file_uploads::Model,
    key: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    let file =
        fetch_completed_file(&req.ctx.db, parse_completed_file_id(upload)?).await?;
    let payload = completed_response(&file, req.upload_id);
    let response = json_endpoint_response(StatusCode::OK, &payload)?;
    let txn = req.ctx.db.begin().await.db_err()?;
    cache_success(&txn, upload.id, ENDPOINT_COMPLETE, key, &response).await?;
    txn.commit().await.db_err()?;
    Ok(response)
}

async fn complete_upload_locked_out(
    req: &CompleteUploadRequest<'_>,
    upload: &file_uploads::Model,
    key: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    let current = load_upload(&req.ctx.db, req.tenant_id, req.upload_id).await?;
    match current.status.as_str() {
        STATUS_COMPLETED => complete_upload_cached(req, &current, key).await,
        STATUS_COMPLETING => Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_busy",
            "upload cannot be completed while completing or completed",
        )),
        STATUS_ABORTED => Err(crate::views::errors::err_custom(
            StatusCode::CONFLICT,
            "upload_aborted",
            "upload has been aborted",
        )),
        STATUS_EXPIRED => Err(crate::views::errors::err_custom(
            StatusCode::GONE,
            "upload_expired",
            "upload has expired",
        )),
        _ => {
            let _ = upload;
            Err(crate::views::errors::err_custom(
                StatusCode::CONFLICT,
                "upload_invalid_state",
                "upload cannot be completed in its current state",
            ))
        }
    }
}

async fn complete_upload_after_lock(
    req: CompleteUploadRequest<'_>,
    mut upload: file_uploads::Model,
    parts: Vec<file_upload_parts::Model>,
    s3_client: &SharedS3Client,
    s3_upload_id: String,
    key: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    if let Err(err) = complete_multipart_upload(
        s3_client,
        &upload.bucket,
        &upload.temp_key,
        &s3_upload_id,
        &parts,
    )
    .await
    {
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_COMPLETING,
            COMPLETE_RETRYABLE_STATUS_REASON,
            false,
        )
        .await;
        return Err(err);
    }

    let streamed_hashes = match stream_object_hashes(
        s3_client,
        &upload.bucket,
        &upload.temp_key,
        upload.expected_size,
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                None,
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                COMPLETE_OBJECT_READ_FAILED_REASON,
                true,
            )
            .await;
            return Err(err);
        }
    };

    if streamed_hashes.actual_size != upload.expected_size as u64 {
        tracing::warn!(
            upload_id = %upload.id,
            expected_size = upload.expected_size,
            actual_size = streamed_hashes.actual_size,
            "uploaded object size mismatch"
        );
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            None,
        )
        .await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            SIZE_MISMATCH_STATUS_REASON,
            true,
        )
        .await;
        return Err(crate::views::errors::err_custom(
            StatusCode::PRECONDITION_FAILED,
            SIZE_MISMATCH_STATUS_REASON,
            "uploaded object size does not match expectedSize",
        ));
    }

    if let Some(declared_hash) = upload.expected_hash.as_deref() {
        if streamed_hashes.full_hash != declared_hash {
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                None,
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                HASH_MISMATCH_STATUS_REASON,
                true,
            )
            .await;
            return Err(crate::views::errors::err_custom(
                StatusCode::PRECONDITION_FAILED,
                "hash_mismatch",
                "server-side BLAKE3 verification failed",
            ));
        }
    } else {
        upload.expected_hash = Some(streamed_hashes.full_hash.clone());
        if let Err(err) = file_uploads::Entity::update_many()
            .col_expr(
                file_uploads::Column::ExpectedHash,
                Expr::value(streamed_hashes.full_hash.clone()),
            )
            .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
            .filter(file_uploads::Column::Id.eq(upload.id))
            .exec(&req.ctx.db)
            .await
        {
            tracing::error!(error = ?err, upload_id = %upload.id, "failed to persist streamed expected_hash");
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                None,
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                COMPLETE_DB_FAILED_REASON,
                true,
            )
            .await;
            return Err(db_err_into(&err));
        }
    }

    let authoritative_hash = upload
        .expected_hash
        .clone()
        .expect("expected_hash set above");

    if let Some(expected_hash_fast) = upload.expected_hash_fast.as_deref() {
        if streamed_hashes.fast_hash.as_deref() != Some(expected_hash_fast) {
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                None,
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                FAST_HASH_MISMATCH_STATUS_REASON,
                true,
            )
            .await;
            return Err(crate::views::errors::err_custom(
                StatusCode::PRECONDITION_FAILED,
                FAST_HASH_MISMATCH_STATUS_REASON,
                "server-side fast hash verification failed",
            ));
        }
    } else if upload.expected_size >= FAST_HASH_THRESHOLD as i64 {
        tracing::warn!(upload_id = %upload.id, "legacy upload completed without expected_hash_fast");
    }

    let detected_mime = detect_mime(&streamed_hashes.mime_sample);
    if is_blacklisted(detected_mime) {
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            None,
        )
        .await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            MIME_BLACKLIST_STATUS_REASON,
            true,
        )
        .await;
        return Err(crate::views::errors::err_custom(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "detected MIME type is blocked for upload",
        ));
    }

    let file_id = upload.id;
    let final_key = final_storage_key(file_id, &authoritative_hash);
    let copy_source = format!("{}/{}", upload.bucket, upload.temp_key);

    if let Err(err) = s3_client
        .copy_object()
        .bucket(upload.bucket.clone())
        .key(final_key.clone())
        .copy_source(copy_source)
        .send()
        .await
    {
        tracing::error!(error = ?err, upload_id = %upload.id, final_key, "failed to copy temp object to final key");
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            Some(&final_key),
        )
        .await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            COMPLETE_COPY_FAILED_REASON,
            true,
        )
        .await;
        return Err(crate::views::errors::err_custom(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
            "failed to copy completed upload to final key",
        ));
    }

    let txn = match req.ctx.db.begin().await {
        Ok(txn) => txn,
        Err(e) => {
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                Some(&final_key),
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_COMPLETING,
                COMPLETE_DB_FAILED_REASON,
                false,
            )
            .await;
            return Err(db_err_into(&e));
        }
    };

    match insert_file_row(
        &txn,
        &upload,
        file_id,
        req.user_id,
        detected_mime,
        streamed_hashes.fast_hash.clone(),
        &final_key,
    )
    .await
    {
        Ok(TryInsertResult::Inserted(_)) => {
            complete_upload_inserted(
                req, txn, upload, file_id, key, s3_client, &final_key,
            )
            .await
        }
        Ok(TryInsertResult::Conflicted | TryInsertResult::Empty) => {
            complete_upload_dedup(
                req,
                txn,
                upload,
                key,
                s3_client,
                &final_key,
                &authoritative_hash,
            )
            .await
        }
        Err(err) => {
            let _ = txn.rollback().await;
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                Some(&final_key),
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                COMPLETE_DB_FAILED_REASON,
                true,
            )
            .await;
            Err(db_err_into(&err))
        }
    }
}

async fn complete_upload_inserted(
    req: CompleteUploadRequest<'_>,
    txn: DatabaseTransaction,
    upload: file_uploads::Model,
    file_id: Uuid,
    key: &str,
    s3_client: &SharedS3Client,
    final_key: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    let inserted_file = match fetch_completed_file(&txn, file_id).await {
        Ok(file) => file,
        Err(err) => {
            let _ = txn.rollback().await;
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                Some(final_key),
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_COMPLETING,
                COMPLETE_DB_FAILED_REASON,
                false,
            )
            .await;
            return Err(err);
        }
    };

    if let Err(err) =
        finalize_inserted_file(&txn, &upload, &inserted_file, req.user_id, req.audit_ctx)
            .await
    {
        let _ = txn.rollback().await;
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            Some(final_key),
        )
        .await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_COMPLETING,
            COMPLETE_DB_FAILED_REASON,
            false,
        )
        .await;
        return Err(err);
    }

    let payload = completed_response(&inserted_file, upload.id);
    let response = json_endpoint_response(StatusCode::OK, &payload)?;
    if let Err(err) =
        cache_success(&txn, upload.id, ENDPOINT_COMPLETE, key, &response).await
    {
        let _ = txn.rollback().await;
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            Some(final_key),
        )
        .await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_COMPLETING,
            COMPLETE_DB_FAILED_REASON,
            false,
        )
        .await;
        return Err(err);
    }

    if let Some(req_attach) = req.attach.as_deref() {
        let req_attach = file_reference_service::AttachRequest {
            file_id: inserted_file.id,
            ..req_attach.clone()
        };
        if let Err(err) =
            file_reference_service::attach_in_txn(&txn, req.audit_ctx, req_attach).await
        {
            let _ = txn.rollback().await;
            cleanup_complete_failure(
                s3_client,
                &upload.bucket,
                upload.id,
                &upload.temp_key,
                Some(final_key),
            )
            .await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_COMPLETING,
                COMPLETE_DB_FAILED_REASON,
                false,
            )
            .await;
            return Err(err);
        }
    }

    if let Err(err) = txn.commit().await {
        cleanup_complete_failure(
            s3_client,
            &upload.bucket,
            upload.id,
            &upload.temp_key,
            Some(final_key),
        )
        .await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            COMPLETE_DB_FAILED_REASON,
            true,
        )
        .await;
        return Err(db_err_into(&err));
    }

    delete_object_if_exists(s3_client, &upload.bucket, &upload.temp_key).await;
    delete_temp_prefix(s3_client, &upload.bucket, upload.id).await;
    Ok(response)
}

async fn complete_upload_dedup(
    req: CompleteUploadRequest<'_>,
    txn: DatabaseTransaction,
    upload: file_uploads::Model,
    key: &str,
    s3_client: &SharedS3Client,
    final_key: &str,
    authoritative_hash: &str,
) -> loco_rs::Result<JsonEndpointResponse> {
    let _ = txn.rollback().await;
    cleanup_complete_failure(
        s3_client,
        &upload.bucket,
        upload.id,
        &upload.temp_key,
        Some(final_key),
    )
    .await;

    let candidate = file_repo::find_any_by_hash_and_size(
        &req.ctx.db,
        upload.tenant_id,
        authoritative_hash,
        upload.expected_size,
    )
    .await?
    .ok_or_else(|| {
        Error::CustomError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorDetail::new(
                "upload.dedup_winner_missing",
                "dedup winner missing after conflict",
            ),
        )
    })?;

    let (winner, _revived) = match file_service::revive_or_use_winner(
        &req.ctx.db,
        candidate,
        req.user_id,
        req.audit_ctx,
    )
    .await
    {
        Ok(pair) => pair,
        Err(err) => {
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                COMPLETE_DB_FAILED_REASON,
                true,
            )
            .await;
            return Err(err);
        }
    };

    let reuse_txn = req.ctx.db.begin().await.db_err()?;
    file_uploads::Entity::update_many()
        .col_expr(file_uploads::Column::Status, Expr::value(STATUS_COMPLETED))
        .col_expr(
            file_uploads::Column::CompletedFileId,
            Expr::value(winner.id),
        )
        .col_expr(
            file_uploads::Column::StatusReason,
            Expr::value(sea_orm::Value::String(None)),
        )
        .col_expr(
            file_uploads::Column::S3UploadId,
            Expr::value(sea_orm::Value::String(None)),
        )
        .col_expr(file_uploads::Column::UpdatedAt, Expr::value(Utc::now()))
        .col_expr(file_uploads::Column::UpdatedBy, Expr::value(req.user_id))
        .filter(file_uploads::Column::Id.eq(upload.id))
        .exec(&reuse_txn)
        .await
        .db_err()?;

    let upload_snapshot = FileUploadAuditSnapshot::from(&upload);
    let file_snapshot = FileAuditSnapshot::from(&winner);
    if let Err(err) = audit_service::log(
        &reuse_txn,
        req.audit_ctx,
        AuditAction::UploadComplete,
        "file_upload",
        &upload.id.to_string(),
        Some(&upload_snapshot),
        Some(&file_snapshot),
    )
    .await
    {
        let _ = reuse_txn.rollback().await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            COMPLETE_DB_FAILED_REASON,
            true,
        )
        .await;
        return Err(err);
    }

    let payload = completed_response(&winner, upload.id);
    let response = json_endpoint_response(StatusCode::OK, &payload)?;
    if let Err(err) =
        cache_success(&reuse_txn, upload.id, ENDPOINT_COMPLETE, key, &response).await
    {
        let _ = reuse_txn.rollback().await;
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            COMPLETE_DB_FAILED_REASON,
            true,
        )
        .await;
        return Err(err);
    }

    if let Some(req_attach) = req.attach.as_deref() {
        let req_attach = file_reference_service::AttachRequest {
            file_id: winner.id,
            ..req_attach.clone()
        };
        if let Err(err) =
            file_reference_service::attach_in_txn(&reuse_txn, req.audit_ctx, req_attach)
                .await
        {
            let _ = reuse_txn.rollback().await;
            best_effort_terminalize_upload_row(
                &req.ctx.db,
                req.upload_id,
                req.user_id,
                STATUS_ABORTED,
                COMPLETE_DB_FAILED_REASON,
                true,
            )
            .await;
            return Err(err);
        }
    }

    if let Err(err) = reuse_txn.commit().await {
        best_effort_terminalize_upload_row(
            &req.ctx.db,
            req.upload_id,
            req.user_id,
            STATUS_ABORTED,
            COMPLETE_DB_FAILED_REASON,
            true,
        )
        .await;
        return Err(db_err_into(&err));
    }

    delete_object_if_exists(s3_client, &upload.bucket, &upload.temp_key).await;
    delete_temp_prefix(s3_client, &upload.bucket, upload.id).await;
    Ok(response)
}
