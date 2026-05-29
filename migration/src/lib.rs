#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::items_after_statements)]
pub use sea_orm_migration::prelude::*;
mod m20220101_000001_users;
mod m20250411_000002_tenants;
mod m20250411_000003_roles;
mod m20250411_000004_permissions;
mod m20250411_000005_user_roles;
mod m20250411_000006_role_permissions;
mod m20250411_000007_sys_menus;
mod m20250411_000008_dict_types;
mod m20250411_000009_dict_items;
mod m20250411_000010_tenant_menu_overrides;
mod m20250411_000011_role_menus;
mod m20250411_000012_sys_role_templates;
mod m20250411_000013_sys_role_template_menus;
mod m20250411_000014_sys_role_template_permissions;
mod m20250419_000015_audit_logs;
mod m20260422_000016_sys_configs;
mod m20260423_000017_files;
mod m20260423_000018_file_uploads;
mod m20260423_000019_file_upload_parts;
mod m20260423_000020_file_references;
mod m20260423_000022_file_upload_idempotency;
mod m20260423_000023_file_instant_idempotency;
mod m20260424_000024_i18n;
mod m20260425_000025_scheduled_workers;
mod m20260503_000026_api_keys;
mod m20260504_000027_seed_system_data;
mod m20260505_000028_notifications;
mod m20260505_000029_seed_notification_data;
mod m20260508_000030_knowledge_base;
mod m20260510_000031_kb_scope_and_chat;
mod m20260512_000032_chat_token_columns;
mod m20260512_000033_session_summary_cache;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_users::Migration),
            Box::new(m20250411_000002_tenants::Migration),
            Box::new(m20250411_000003_roles::Migration),
            Box::new(m20250411_000004_permissions::Migration),
            Box::new(m20250411_000005_user_roles::Migration),
            Box::new(m20250411_000006_role_permissions::Migration),
            Box::new(m20250411_000007_sys_menus::Migration),
            Box::new(m20250411_000008_dict_types::Migration),
            Box::new(m20250411_000009_dict_items::Migration),
            Box::new(m20250411_000010_tenant_menu_overrides::Migration),
            Box::new(m20250411_000011_role_menus::Migration),
            Box::new(m20250411_000012_sys_role_templates::Migration),
            Box::new(m20250411_000013_sys_role_template_menus::Migration),
            Box::new(m20250411_000014_sys_role_template_permissions::Migration),
            Box::new(m20250419_000015_audit_logs::Migration),
            Box::new(m20260422_000016_sys_configs::Migration),
            Box::new(m20260423_000017_files::Migration),
            Box::new(m20260423_000018_file_uploads::Migration),
            Box::new(m20260423_000019_file_upload_parts::Migration),
            Box::new(m20260423_000020_file_references::Migration),
            Box::new(m20260423_000022_file_upload_idempotency::Migration),
            Box::new(m20260423_000023_file_instant_idempotency::Migration),
            Box::new(m20260424_000024_i18n::Migration),
            Box::new(m20260425_000025_scheduled_workers::Migration),
            Box::new(m20260503_000026_api_keys::Migration),
            Box::new(m20260504_000027_seed_system_data::Migration),
            Box::new(m20260505_000028_notifications::Migration),
            Box::new(m20260505_000029_seed_notification_data::Migration),
            Box::new(m20260508_000030_knowledge_base::Migration),
            Box::new(m20260510_000031_kb_scope_and_chat::Migration),
            Box::new(m20260512_000032_chat_token_columns::Migration),
            Box::new(m20260512_000033_session_summary_cache::Migration),
            // inject-above (do not remove this comment)
        ]
    }
}
