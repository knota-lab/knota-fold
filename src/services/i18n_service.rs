//! Facade re-exporting all i18n sub-modules.
//!
//! External callers use `i18n_service::fn_name` unchanged.

// ── Bundle resolution, ETag, cache, revision ──────────────────────────────
pub use crate::services::i18n_bundle::bump_global_revision_pub;
pub use crate::services::i18n_bundle::bump_tenant_revision_pub;
pub use crate::services::i18n_bundle::compute_etag;
pub use crate::services::i18n_bundle::etag_for;
pub use crate::services::i18n_bundle::invalidate_global_bundle_cache;
pub use crate::services::i18n_bundle::resolve_bundle;

// ── Translation key listing ───────────────────────────────────────────────
pub use crate::services::i18n_key_list::list_global_keys;
pub use crate::services::i18n_key_list::list_namespaces;
pub use crate::services::i18n_key_list::list_tenant_keys;
pub use crate::services::i18n_key_list::list_tenant_namespaces;

// ── Single-record CRUD ────────────────────────────────────────────────────
pub use crate::services::i18n_crud::delete_global_translation_by_id;
pub use crate::services::i18n_crud::delete_tenant_override_by_id;
pub use crate::services::i18n_crud::delete_tenant_override_by_triple;
pub(crate) use crate::services::i18n_crud::force_sync_global_in_txn;
pub use crate::services::i18n_crud::update_global_translation_by_id;
pub use crate::services::i18n_crud::upsert_global_translation;
pub use crate::services::i18n_crud::upsert_tenant_override;
pub(crate) use crate::services::i18n_crud::ForceSyncOutcome;

// ── Import / export ───────────────────────────────────────────────────────
pub use crate::services::i18n_import_export::export_global;
pub use crate::services::i18n_import_export::export_tenant;
pub use crate::services::i18n_import_export::import_global;
pub use crate::services::i18n_import_export::import_tenant;

// ── Batch update (update-only) ────────────────────────────────────────────
pub use crate::services::i18n_crud::batch_update_global;
pub use crate::services::i18n_crud::batch_update_tenant;

// ── Entry management ──────────────────────────────────────────────────────
pub use crate::services::i18n_entries::delete_entry_cascade;
pub use crate::services::i18n_entries::list_entries;
pub use crate::services::i18n_entries::list_entry_locations;
