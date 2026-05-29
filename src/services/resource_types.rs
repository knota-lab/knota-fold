//! Resource type registry for file references.
//!
//! Every entity that can have files attached registers itself here as an
//! [`ResourceType`] variant. The wire / DB representation is the
//! `domain:entity` string (e.g. `"crm:contract"`).
//!
//! Why an enum (not raw strings):
//! - Compile-time prevention of typos in attach / detach call sites.
//! - `grep` for a variant instantly enumerates every consumer.
//! - Adding a new attachable entity requires touching this file, which
//!   forces a code review of the integration.
//!
//! DB schema still stores `VARCHAR(64)` to allow forward-compatible reads
//! when a variant is retired: legacy rows can still be selected as
//! `String` and surfaced to ops, instead of panicking.
//!
//! ## Registered variants
//!
//! - [`ResourceType::SystemAttachment`] — "standalone uploads" surfaced
//!   by the admin attachments page (`/files`). Every file uploaded via
//!   that page attaches to itself: `resource_id == file_id`. This makes
//!   the file its own owning business entity, so it is no longer an
//!   orphan from the reference graph's perspective and the same
//!   detach → purge lifecycle works uniformly.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Strongly-typed resource kind for [`file_references`](crate::models::file_references).
///
/// Serialization uses the canonical `domain:entity` string form, matching
/// what is stored in `file_references.resource_type`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    /// Standalone upload via the admin attachments page. The file is its
    /// own owning entity, so callers MUST set `resource_id == file_id`
    /// when attaching. The [`file_reference_service`] currently does NOT
    /// enforce that invariant at the DB layer (the column is just text);
    /// the convention is enforced by the upload entry points that pass
    /// `attach_to`.
    ///
    /// Detach semantics (admin "delete" button) follow the standard
    /// reference-driven lifecycle:
    /// 1. Soft-detach the `file_references` row (sets `deleted_at`).
    /// 2. The `purge_files` background task picks the file up after
    ///    both grace windows elapse (`GRACE_PERIOD_HOURS` for the file
    ///    itself + `REFERENCE_DETACH_GRACE_HOURS` for the detach event)
    ///    and hard-deletes the S3 object + DB row.
    #[serde(rename = "system:attachment")]
    SystemAttachment,
    /// Placeholder for the future CRM contract module. Registered ahead
    /// of the actual feature so integration tests can exercise multi-type
    /// filter / detach paths against `file_references`. Will be promoted
    /// to a real consumer when the CRM module lands; until then it is a
    /// no-op variant — no controller writes it in production.
    #[serde(rename = "crm:contract")]
    CrmContract,
    // Add variants when more business modules start attaching files.
    // Naming convention: `domain:entity`, where `domain` namespaces the
    // bounded context (system / iam / ops / crm / ...).
}

impl ResourceType {
    /// Canonical wire / DB representation.
    ///
    /// Must match the `#[serde(rename = ...)]` exactly; the registry
    /// test `as_str_matches_serde_rename` enforces this invariant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemAttachment => "system:attachment",
            Self::CrmContract => "crm:contract",
        }
    }

    /// Parse from DB / wire string. Returns [`UnknownResourceType`] for
    /// unregistered values so callers can decide between `400 Bad Request`
    /// (attach API) and lossy passthrough (read APIs).
    ///
    /// # Errors
    ///
    /// Returns [`UnknownResourceType`] when `s` is not a registered variant.
    pub fn parse(s: &str) -> Result<Self, UnknownResourceType> {
        match s {
            "system:attachment" => Ok(Self::SystemAttachment),
            "crm:contract" => Ok(Self::CrmContract),
            other => Err(UnknownResourceType(other.to_owned())),
        }
    }

    /// Whitelist check used by attach API to reject unknown types early
    /// (4xx) instead of letting typos persist in the database.
    #[must_use]
    pub fn is_known(s: &str) -> bool {
        Self::parse(s).is_ok()
    }
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned by [`ResourceType::parse`] when the input string is not a
/// registered variant. Callers typically translate this into a
/// `400 Bad Request` with the offending value echoed back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownResourceType(pub String);

impl fmt::Display for UnknownResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown resource_type: {}", self.0)
    }
}

impl std::error::Error for UnknownResourceType {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against drift between [`ResourceType::as_str`] and the
    /// `#[serde(rename = ...)]` attributes. If they ever diverge, DB rows
    /// written by one path become unparseable by the other.
    #[test]
    fn as_str_matches_serde_rename() {
        for variant in [ResourceType::SystemAttachment, ResourceType::CrmContract] {
            let serialized = serde_json::to_string(&variant).expect("serialize");
            // serde_json wraps strings in quotes
            let unquoted = serialized.trim_matches('"');
            assert_eq!(
                unquoted,
                variant.as_str(),
                "as_str() must match serde rename for {variant:?}"
            );
        }
    }

    #[test]
    fn parse_roundtrips() {
        for variant in [ResourceType::SystemAttachment, ResourceType::CrmContract] {
            assert_eq!(ResourceType::parse(variant.as_str()), Ok(variant));
        }
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(ResourceType::parse("system:nope").is_err());
        assert!(ResourceType::parse("").is_err());
        assert!(!ResourceType::is_known("garbage"));
    }

    /// `UnknownResourceType` echoes the offending string so logs and
    /// 400 responses can show what the caller actually sent.
    #[test]
    fn unknown_error_preserves_input() {
        let err = ResourceType::parse("ops:unregistered").unwrap_err();
        assert_eq!(err.0, "ops:unregistered");
        assert_eq!(err.to_string(), "unknown resource_type: ops:unregistered");
    }
}
