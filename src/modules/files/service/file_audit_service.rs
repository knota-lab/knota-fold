//! File-audit service — Wave 1 skeleton (signatures only).
//!
//! Thin facade over [`crate::services::audit_service::log`] for the file
//! domain, keeping action enum + snapshot wiring in one place.
//! Real implementation arrives in Wave 2; bodies intentionally `todo!()`.

use sea_orm::ConnectionTrait;

use crate::models::_entities::{file_references, file_uploads, files};
use crate::views::audit_logs::AuditContext;

#[tracing::instrument(skip_all)]
pub async fn log_upload_complete<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _upload_before: &file_uploads::Model,
    _file_after: &files::Model,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}

#[tracing::instrument(skip_all)]
pub async fn log_upload_abort<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _upload_before: &file_uploads::Model,
    _reason: &str,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}

#[tracing::instrument(skip_all)]
pub async fn log_purge<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _file_before: &files::Model,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}

#[tracing::instrument(skip_all)]
pub async fn log_soft_delete<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _file_before: &files::Model,
    _file_after: &files::Model,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}

#[tracing::instrument(skip_all)]
pub async fn log_restore<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _file_before: &files::Model,
    _file_after: &files::Model,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}

#[tracing::instrument(skip_all)]
pub async fn log_reference<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _ref_after: &file_references::Model,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}

#[tracing::instrument(skip_all)]
pub async fn log_dereference<C: ConnectionTrait>(
    _db: &C,
    _audit_ctx: &AuditContext,
    _ref_before: &file_references::Model,
) -> loco_rs::Result<()> {
    todo!("Wave 2")
}
