use knota_fold::app::App;
use loco_rs::cli;
use migration::Migrator;

#[tokio::main]
async fn main() -> loco_rs::Result<()> {
    // Load .env file (Vite-style). No-op if file doesn't exist.
    let _ = dotenvy::dotenv();

    cli::main::<App, Migrator>().await
}
