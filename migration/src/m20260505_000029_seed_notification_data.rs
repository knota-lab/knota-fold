use sea_orm::DatabaseBackend;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Re-seed sys_menus and permissions from the updated YAML fixtures.
        // ON CONFLICT DO NOTHING makes this idempotent: existing rows are
        // skipped, only new notification entries are inserted.
        seed(
            manager,
            "sys_menus",
            include_str!("../../src/fixtures/sys_menus.yaml"),
        )
        .await?;
        seed(
            manager,
            "permissions",
            include_str!("../../src/fixtures/permissions.yaml"),
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove notification menu entries (directory + child menu)
        for id in [
            "cccccccc-cccc-cccc-cccc-ccccccccc121",
            "cccccccc-cccc-cccc-cccc-ccccccccc120",
        ] {
            manager
                .get_connection()
                .execute_unprepared(&format!(
                    "DELETE FROM sys_menus WHERE id = X'{}'",
                    id.replace('-', "").to_uppercase()
                ))
                .await?;
        }

        // Remove notification permission entries
        for id in [
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb150",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb151",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbb152",
        ] {
            manager
                .get_connection()
                .execute_unprepared(&format!(
                    "DELETE FROM permissions WHERE id = X'{}'",
                    id.replace('-', "").to_uppercase()
                ))
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
    let backend = {
        let conn = manager.get_connection();
        conn.get_database_backend()
    };

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
            if backend == DatabaseBackend::Sqlite && looks_like_uuid(string) {
                uuid_str_to_hex_blob(string)
            } else {
                format!("'{}'", string.replace('\'', "''"))
            }
        }
        _ => format!("'{}'", value.as_str().unwrap_or("").replace('\'', "''")),
    }
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4
}

fn uuid_str_to_hex_blob(s: &str) -> String {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    format!("X'{}'", hex.to_uppercase())
}
