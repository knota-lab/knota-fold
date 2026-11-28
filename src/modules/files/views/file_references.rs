//! File reference DTOs.
//!
//! `file_references` rows tie a file to a row in some business table
//! (e.g. a `dict_item`). The wire shape mirrors what UI consumers need:
//! the reference identity (`id`), the linked file (`fileId`), the
//! resource it is attached to, the optional `fieldName` discriminator
//! when the same file is reused across columns of the same row, and
//! provenance metadata (`createdBy` / `createdAt`).
//!
//! Field naming follows the project-wide camelCase convention for HTTP
//! payloads.

use chrono::{DateTime, FixedOffset};
use loco_openapi::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::_entities::file_references;
use crate::views::files::FileResponse;

/// Caller-supplied attach payload.
///
/// `fieldName` defaults to `""` (single-attachment slot per resource);
/// non-empty values let the same file appear under multiple form fields
/// of the same business row without violating the active-row uniqueness
/// constraint `(tenant, file, resource_type, resource_id, field_name)`.
///
/// `displayName` is a UI-only label snapshot. It is not used for
/// uniqueness, so renaming the attachment never produces a duplicate
/// row. When attaching, an empty/`None` value falls back to the file's
/// canonical `name` at attach time.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachReferenceRequest {
    /// Strongly-typed resource kind (`domain:entity`). Unknown values
    /// are rejected at the controller boundary before reaching the DB.
    pub resource_type: String,
    pub resource_id: String,
    #[serde(default)]
    pub field_name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Response shape for a single `file_references` row. Active rows only;
/// soft-deleted rows are filtered at the service layer and never reach
/// the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReferenceResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub file_id: Uuid,
    pub resource_type: String,
    pub resource_id: String,
    pub field_name: String,
    pub display_name: Option<String>,
    pub mime_type: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<FixedOffset>,
}

impl From<file_references::Model> for FileReferenceResponse {
    fn from(m: file_references::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            file_id: m.file_id,
            resource_type: m.resource_type,
            resource_id: m.resource_id,
            field_name: m.field_name,
            display_name: m.display_name,
            mime_type: m.mime_type,
            created_by: m.created_by,
            created_at: m.created_at,
        }
    }
}

/// Joined response: a `file_references` row plus the underlying `files` row
/// inlined under `file`.
///
/// Powers the admin "all attachments" view where the primary entity is the
/// business attachment (one row per attach event), and `file.*` is metadata
/// about the physical bytes.
///
/// `file` is `Option` because the cleanup task may have hard-purged
/// the underlying file row while a soft-deleted reference lingers
/// (defensive — the list endpoint filters `deleted_at IS NULL` so this
/// should be `Some` in practice; we still surface the case rather
/// than 500 the request).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReferenceWithFileResponse {
    #[serde(flatten)]
    pub reference: FileReferenceResponse,
    pub file: Option<FileResponse>,
}
