pub mod _entities;
pub mod api_key_exchange_tokens;
pub mod api_keys;
pub mod audit_logs;
pub mod dict_items;
pub mod dict_types;
pub mod file_instant_idempotency;
pub mod file_references;
pub mod file_repo;
pub mod file_upload_idempotency;
pub mod file_upload_parts;
pub mod file_uploads;
pub mod files;
pub mod i18n_bundle_revisions;
pub mod i18n_entries;
pub mod i18n_entry_locations;
pub mod i18n_queries;
pub mod i18n_supported_locales;
pub mod i18n_translations;
pub mod permissions;
pub mod roles;
pub mod scheduled_worker_definitions;
pub mod scheduled_worker_executions;
pub mod scheduled_worker_schedules;
pub mod scheduled_worker_tenant_grants;
pub mod sys_configs;
pub mod sys_menus;
pub mod sys_role_templates;
pub mod tenant_menu_overrides;
pub mod tenants;
pub mod users;

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
