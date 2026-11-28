// Compatibility re-exports. Canonical paths live under `crate::modules`.
pub mod _entities;

pub use crate::modules::api_keys::models::{api_key_exchange_tokens, api_keys};
pub use crate::modules::audit_logs::models::audit_logs;
pub use crate::modules::dicts::models::{dict_items, dict_types};
pub use crate::modules::files::models::{
    file_instant_idempotency, file_references, file_repo, file_upload_idempotency,
    file_upload_parts, file_uploads, files,
};
pub use crate::modules::i18n::models::{
    i18n_bundle_revisions, i18n_entries, i18n_entry_locations, i18n_queries,
    i18n_supported_locales, i18n_translations,
};
pub use crate::modules::menus::models::{sys_menus, tenant_menu_overrides};
pub use crate::modules::permissions::models::permissions;
pub use crate::modules::roles::models::{roles, sys_role_templates};
pub use crate::modules::sys_configs::models::sys_configs;
pub use crate::modules::tenants::models::tenants;
pub use crate::modules::users::models::users;
pub use crate::modules::workers::models::{
    scheduled_worker_definitions, scheduled_worker_executions,
    scheduled_worker_schedules, scheduled_worker_tenant_grants,
};

impl sea_orm::ActiveModelBehavior for _entities::role_menus::ActiveModel {}
impl sea_orm::ActiveModelBehavior for _entities::role_permissions::ActiveModel {}
impl sea_orm::ActiveModelBehavior for _entities::user_roles::ActiveModel {}
impl sea_orm::ActiveModelBehavior for _entities::sys_role_template_menus::ActiveModel {}
impl sea_orm::ActiveModelBehavior
    for _entities::sys_role_template_permissions::ActiveModel
{
}
// Notification entities — ActiveModelBehavior is implemented in modules/notification/models/
// notification_recipients has no custom behavior, needs empty impl here.
impl sea_orm::ActiveModelBehavior for _entities::notification_recipients::ActiveModel {}
