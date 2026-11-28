// Compatibility re-exports. Canonical paths live under `crate::modules`.
pub mod errors;
pub mod pagination;

pub use crate::modules::api_keys::views::{api_key_exchange_tokens, api_keys};
pub use crate::modules::audit_logs::views::audit_logs;
pub use crate::modules::auth::views::auth;
pub use crate::modules::dicts::views::dicts;
pub use crate::modules::files::views::{
    file_audit, file_references, file_uploads, files,
};
pub use crate::modules::i18n::views::i18n;
pub use crate::modules::menus::views::{menus, sys_menus};
pub use crate::modules::permissions::views::permissions;
pub use crate::modules::roles::views::{role_templates, roles};
pub use crate::modules::sys_configs::views::sys_configs;
pub use crate::modules::tenants::views::tenants;
pub use crate::modules::users::views::users;
pub use crate::modules::workers::views::worker_scheduler;
