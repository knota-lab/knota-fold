//! Wave 1 DoD fallback verifier — counts seeded file-management permissions.
//!
//! Background: Wave 1 计划 (`.sisyphus/plans/file-management-rust-wave1.md`
//! L249-290) defines this example as the deterministic fallback when
//! `sqlite3.exe` is not on PATH. It loads the runtime config, opens the
//! configured DB via the same `boot::create_context` path the server uses,
//! and asserts that exactly **36** file-domain permissions exist after
//! `db reset` + `db seed` (Track 1 ×18 + Track 2 ×18; design §14.1).
//!
//! Run:
//! ```powershell
//! cargo run --example check_file_permissions
//! ```
//! Expected stdout: `file_permissions_count = 36` and exit code 0.

use knota_fold::app::App;
use knota_fold::models::_entities::permissions;
use loco_rs::boot::create_context;
use loco_rs::environment::{resolve_from_env, Environment};
use sea_orm::{ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter};

#[tokio::main]
async fn main() -> loco_rs::Result<()> {
    // 1. Resolve environment (LOCO_ENV / RAILS_ENV / NODE_ENV → default "development").
    //    Source: loco-rs 0.16 src/environment.rs#L28-L35.
    let environment: Environment = resolve_from_env().into();

    // 2. Load config from config/{env}.yaml.
    //    Source: loco-rs 0.16 src/environment.rs#L48-L57.
    let config = environment.load()?;

    // 3. Create AppContext (boot::create_context — environment + config).
    //    Source: loco-rs 0.16 src/boot.rs#L351-L365.
    let ctx = create_context::<App>(&environment, config).await?;

    // 4. Count file-domain permissions across all 6 route families
    //    (Track 1: files, file-uploads, business reverse-lookup;
    //     Track 2: sys/tenants/{code}/files, /file-uploads, /business reverse-lookup).
    let total = permissions::Entity::find()
        .filter(
            Condition::any()
                .add(permissions::Column::Code.like("%/api/files%"))
                .add(permissions::Column::Code.like("%/api/file-uploads%"))
                .add(permissions::Column::Code.like("%/api/business/%/files"))
                .add(permissions::Column::Code.like("%/api/sys/tenants/%/files%"))
                .add(permissions::Column::Code.like("%/api/sys/tenants/%/file-uploads%"))
                .add(
                    permissions::Column::Code
                        .like("%/api/sys/tenants/%/business/%/files"),
                ),
        )
        .count(&ctx.db)
        .await?;

    println!("file_permissions_count = {total}");
    assert_eq!(total, 36, "expected 36 file permissions, got {total}");
    Ok(())
}
