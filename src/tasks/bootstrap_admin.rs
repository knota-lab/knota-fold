//! Bootstrap super-admin user task.
//!
//! Creates a single admin user and assigns the `SUPER_ADMIN` role.
//! Idempotent — if the email already exists the task exits silently.
//!
//! # Usage
//!
//! ```sh
//! # Via CLI args
//! knota_fold-cli task bootstrap_admin email:admin@example.com password:secret123
//!
//! # Via environment variables (preferred in Docker)
//! BOOTSTRAP_ADMIN_EMAIL=admin@example.com \
//! BOOTSTRAP_ADMIN_PASSWORD=secret123 \
//! knota_fold-cli task bootstrap_admin
//! ```
//!
//! Optional args / env vars:
//! - `name` / `BOOTSTRAP_ADMIN_NAME` — display name (default: "Super Admin")
//! - `tenant_id` / `BOOTSTRAP_ADMIN_TENANT_ID` — tenant UUID (default: system tenant)

use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{self, Task, TaskInfo},
    Result,
};
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::models::_entities::{roles, user_roles, users};
use crate::models::users::RegisterParams;

pub struct BootstrapAdmin;

const DEFAULT_TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";
const SUPER_ADMIN_ROLE_CODE: &str = "SUPER_ADMIN";

#[async_trait]
impl Task for BootstrapAdmin {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "bootstrap_admin".to_string(),
            detail: "Create super admin user (idempotent). Args: email, password, [name]"
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        // ── Resolve parameters: CLI arg > env var > default ──────
        let email = resolve(vars, "email", "BOOTSTRAP_ADMIN_EMAIL").ok_or_else(|| {
            loco_rs::Error::string(
                "email is required: pass email:xxx as CLI arg or set BOOTSTRAP_ADMIN_EMAIL",
            )
        })?;

        let password = resolve(vars, "password", "BOOTSTRAP_ADMIN_PASSWORD").ok_or_else(|| {
            loco_rs::Error::string(
                "password is required: pass password:xxx as CLI arg or set BOOTSTRAP_ADMIN_PASSWORD",
            )
        })?;

        let name = resolve(vars, "name", "BOOTSTRAP_ADMIN_NAME")
            .unwrap_or_else(|| "Super Admin".to_string());

        let tenant_id_str = resolve(vars, "tenant_id", "BOOTSTRAP_ADMIN_TENANT_ID")
            .unwrap_or_else(|| DEFAULT_TENANT_ID.to_string());
        let tenant_id = Uuid::parse_str(&tenant_id_str)
            .map_err(|_| loco_rs::Error::string("invalid tenant_id"))?;

        // ── Idempotency check ────────────────────────────────────
        let existing = users::Entity::find()
            .filter(users::Column::Email.eq(&email))
            .one(&ctx.db)
            .await?;

        if let Some(user) = existing {
            tracing::info!(email = %user.email, "Admin user already exists — skipping");
            return Ok(());
        }

        // ── Create user ──────────────────────────────────────────
        let user = users::Model::create_with_password(
            &ctx.db,
            &RegisterParams {
                email,
                password,
                name,
                tenant_id: Some(tenant_id),
            },
        )
        .await
        .map_err(|e| {
            loco_rs::Error::string(&format!("failed to create admin user: {e}"))
        })?;

        tracing::info!(user_id = %user.id, email = %user.email, "Admin user created");

        // ── Assign SUPER_ADMIN role ──────────────────────────────
        let role = roles::Entity::find()
            .filter(roles::Column::Code.eq(SUPER_ADMIN_ROLE_CODE))
            .one(&ctx.db)
            .await?
            .ok_or_else(|| {
                loco_rs::Error::string(
                    "SUPER_ADMIN role not found — run `task seed_system` first",
                )
            })?;

        let user_role = user_roles::ActiveModel {
            tenant_id: ActiveValue::set(user.tenant_id),
            user_id: ActiveValue::set(user.id),
            role_id: ActiveValue::set(role.id),
        };
        user_role.insert(&ctx.db).await?;

        tracing::info!(role = %role.code, "SUPER_ADMIN role assigned");

        Ok(())
    }
}

/// Resolve a parameter from CLI args, falling back to an env var.
fn resolve(vars: &task::Vars, arg: &str, env: &str) -> Option<String> {
    vars.cli_arg(arg)
        .ok()
        .cloned()
        .or_else(|| std::env::var(env).ok())
}
