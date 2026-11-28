// Compatibility re-exports. Canonical paths live under `crate::modules`.
pub mod casbin_service;
pub mod resource_types;

pub use crate::modules::api_keys::service::api_key_service;
pub use crate::modules::audit_logs::service::audit_service;
pub use crate::modules::auth::service::{
    auth_cache, auth_policy, captcha_service, login_guard,
};
pub use crate::modules::dicts::service::dict_service;
pub use crate::modules::files::service::{
    file_audit_service, file_reference_service, file_service, file_upload_service,
    partition_policy,
};
pub use crate::modules::i18n::service::{
    i18n_bundle, i18n_crud, i18n_entries, i18n_import_export, i18n_key_list,
    i18n_locale_service, i18n_manifest_service, i18n_service, i18n_validation,
};
pub use crate::modules::menus::service::{sys_menu_service, tenant_menu_service};
pub use crate::modules::permissions::service::permission_service;
pub use crate::modules::roles::service::{role_service, role_template_service};
pub use crate::modules::sys_configs::service::sys_config_service;
pub use crate::modules::tenants::service::tenant_service;
pub use crate::modules::users::service::user_service;
pub use crate::modules::workers::service::{
    task_execution_service, task_scheduler_service, worker_run_service,
};
