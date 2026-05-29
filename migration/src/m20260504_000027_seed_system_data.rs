use sea_orm::DatabaseBackend;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        seed(
            manager,
            "tenants",
            include_str!("../../src/fixtures/tenants.yaml"),
        )
        .await?;
        seed(
            manager,
            "roles",
            include_str!("../../src/fixtures/roles.yaml"),
        )
        .await?;
        seed(
            manager,
            "permissions",
            include_str!("../../src/fixtures/permissions.yaml"),
        )
        .await?;
        seed(
            manager,
            "role_permissions",
            include_str!("../../src/fixtures/role_permissions.yaml"),
        )
        .await?;
        seed(
            manager,
            "sys_menus",
            include_str!("../../src/fixtures/sys_menus.yaml"),
        )
        .await?;
        seed(
            manager,
            "dict_types",
            include_str!("../../src/fixtures/dict_types.yaml"),
        )
        .await?;
        seed(
            manager,
            "dict_items",
            include_str!("../../src/fixtures/dict_items.yaml"),
        )
        .await?;
        seed(
            manager,
            "sys_configs",
            include_str!("../../src/fixtures/sys_configs.yaml"),
        )
        .await?;
        seed(
            manager,
            "scheduled_worker_definitions",
            include_str!("../../src/fixtures/scheduled_worker_definitions.yaml"),
        )
        .await?;
        seed(
            manager,
            "scheduled_worker_tenant_grants",
            include_str!("../../src/fixtures/scheduled_worker_tenant_grants.yaml"),
        )
        .await?;
        seed(
            manager,
            "sys_role_templates",
            include_str!("../../src/fixtures/sys_role_templates.yaml"),
        )
        .await?;
        seed(
            manager,
            "sys_role_template_permissions",
            include_str!("../../src/fixtures/sys_role_template_permissions.yaml"),
        )
        .await?;
        seed(
            manager,
            "sys_role_template_menus",
            include_str!("../../src/fixtures/sys_role_template_menus.yaml"),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "sys_role_template_menus",
            "sys_role_template_permissions",
            "sys_role_templates",
            "scheduled_worker_tenant_grants",
            "scheduled_worker_definitions",
            "sys_configs",
            "dict_items",
            "dict_types",
            "sys_menus",
            "role_permissions",
            "permissions",
            "roles",
            "tenants",
        ] {
            manager
                .get_connection()
                .execute_unprepared(&format!("DELETE FROM {table}"))
                .await?;
        }

        Ok(())
    }
}

/// Insert seed rows from a YAML fixture into the given table.
///
/// Uses `INSERT INTO ... ON CONFLICT DO NOTHING` so the migration is
/// idempotent — safe to re-run after `cargo loco db seed --reset`.
async fn seed(manager: &SchemaManager<'_>, table: &str, yaml: &str) -> Result<(), DbErr> {
    let backend = manager.get_connection().get_database_backend();

    let rows: Vec<serde_yaml::Mapping> = serde_yaml::from_str(yaml)
        .map_err(|e| DbErr::Custom(format!("parse {table}: {e}")))?;

    for row in rows {
        let mut cols = Vec::with_capacity(row.len());
        let mut vals = Vec::with_capacity(row.len());

        for (k, v) in &row {
            let key = k
                .as_str()
                .ok_or_else(|| DbErr::Custom(format!("parse {table}: non-string key")))?;

            let col = remap_column(table, key);
            cols.push(format!("\"{col}\""));
            vals.push(yaml_value_to_sql(v, backend));
        }

        // ON CONFLICT DO NOTHING — idempotent, won't fail if data exists
        let sql = format!(
            "INSERT INTO {table} ({}) VALUES ({}) ON CONFLICT DO NOTHING",
            cols.join(", "),
            vals.join(", ")
        );

        manager.get_connection().execute_unprepared(&sql).await?;
    }

    Ok(())
}

/// Map Rust field names in YAML fixtures to actual DB column names.
///
/// YAML fixtures are shared with `cargo loco db seed` which uses `SeaORM`
/// entity deserialization (Rust field names).  This migration generates
/// raw SQL so we must translate back to DB column names here.
fn remap_column<'a>(table: &str, key: &'a str) -> &'a str {
    match (table, key) {
        ("permissions", "permission_type") | ("sys_menus", "menu_type") => "type",
        _ => key,
    }
}

fn yaml_value_to_sql(value: &serde_yaml::Value, backend: DatabaseBackend) -> String {
    match value {
        serde_yaml::Value::Null => "NULL".to_string(),
        serde_yaml::Value::Bool(true) => "TRUE".to_string(),
        serde_yaml::Value::Bool(false) => "FALSE".to_string(),
        serde_yaml::Value::Number(number) => number.to_string(),
        serde_yaml::Value::String(string) => {
            // SQLite stores UUID columns as 16-byte BLOBs.  A raw SQL string
            // like 'aaaa...' (36 chars) would be stored as TEXT, causing
            // `ParseByteLength { len: 36 }` when SeaORM reads it back.
            // PostgreSQL has native UUID support and expects the string form.
            if backend == DatabaseBackend::Sqlite && looks_like_uuid(string) {
                uuid_str_to_hex_blob(string)
            } else {
                format!("'{}'", string.replace('\'', "''"))
            }
        }
        _ => format!("'{}'", value.as_str().unwrap_or("").replace('\'', "''")),
    }
}

/// Check if a string looks like a UUID (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx).
fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4
}

/// Convert UUID string to `SQLite` hex-blob literal: X'0123...EF'
///
/// `"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaa001"` → `X'AAAAAAAAAAAAAAA AAAAAAAAAA AAAAAAAA AAAAAAAA AAAAAAA001'`
fn uuid_str_to_hex_blob(s: &str) -> String {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    // Already lowercase hex from the UUID; force uppercase for readability.
    format!("X'{}'", hex.to_uppercase())
}
