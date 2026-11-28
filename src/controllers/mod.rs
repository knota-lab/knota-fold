// Compatibility re-exports. Canonical paths live under `crate::modules`.

pub use crate::modules::api_keys::controller::{api_key_exchange_tokens, api_keys};
pub use crate::modules::audit_logs::controller::audit_logs;
pub use crate::modules::auth::controller::auth;
pub use crate::modules::dicts::controller::dicts;
pub use crate::modules::files::controller::{
    file_references, file_uploads, files, sys_file_references, sys_file_uploads,
    sys_files,
};
pub use crate::modules::i18n::controller::{admin_i18n, ci_i18n, i18n, tenant_i18n};
pub use crate::modules::menus::controller::{menus, sys_menus};
pub use crate::modules::permissions::controller::permissions;
pub use crate::modules::roles::controller::{role_templates, roles};
pub use crate::modules::sys_configs::controller::sys_configs;
pub use crate::modules::tenants::controller::tenants;
pub use crate::modules::users::controller::users;
pub use crate::modules::workers::controller::{
    worker_definitions, worker_executions, worker_schedules,
};
