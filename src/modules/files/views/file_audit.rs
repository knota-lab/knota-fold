//! File-domain audit DTOs.
//!
//! Snapshot structs themselves live in [`crate::modules::audit_logs::views::audit_logs`] alongside
//! the rest of the audit machinery (see `FileAuditSnapshot` and
//! `FileUploadAuditSnapshot`). This module re-exports them so callers can
//! `use crate::modules::files::views::file_audit::*` for symmetry with other view modules.

pub use crate::modules::audit_logs::views::audit_logs::{
    FileAuditSnapshot, FileReferenceAuditSnapshot, FileUploadAuditSnapshot,
};
