pub mod config;
pub mod format;
pub mod layer;
pub mod routes;
pub mod schema;
pub mod writer;

use async_trait::async_trait;
use loco_rs::{app::AppContext, Result};

use crate::config::ConfigExt;

pub struct AppLogsInitializer;

#[async_trait]
impl loco_rs::app::Initializer for AppLogsInitializer {
    fn name(&self) -> String {
        "app-logs".to_string()
    }

    async fn before_run(&self, _ctx: &AppContext) -> Result<()> {
        // All initialization happens in init_logger (called before initializers).
        // This hook is intentionally left empty.
        Ok(())
    }
}

/// Called from `Hooks::init_logger`. Sets up DB, channel, writer thread,
/// and tracing Layer — all before loco's default logger would run.
///
/// Returns `Ok(true)` to tell loco "I've handled logging, don't init yours."
pub fn init_logger(ctx: &AppContext) -> Result<bool> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let typed = ctx.config.typed_settings().ok().flatten();
    let app_logs_cfg = typed.as_ref().and_then(|s| s.app_logs.as_ref());
    let enabled = app_logs_cfg.is_some_and(|c| c.enabled);

    // Build the same stdout layer + env filter that loco's logger::init would.
    // Note: intentionally NOT using FmtSpan::CLOSE because loco-rs's built-in
    // logger middleware uses `tracing::error_span!` for request spans, which
    // would cause every request to print an ERROR-level span-close line.
    let fmt_layer =
        tracing_subscriber::fmt::layer().event_format(format::BusinessLocationFormat);

    // Build filter: RUST_LOG > config override_filter > config level + whitelist.
    // Rust module paths use underscores, which is also what CARGO_CRATE_NAME
    // exposes for a hyphenated package name.
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| {
            ctx.config.logger.override_filter.as_ref().map_or_else(
                || {
                    EnvFilter::try_new(format!(
                        "loco_rs={lvl},sea_orm_migration={lvl},tower_http={lvl},sqlx::query={lvl},{}={lvl}",
                        env!("CARGO_CRATE_NAME"),
                        lvl = ctx.config.logger.level
                    ))
                },
                EnvFilter::try_new,
            )
        })
        .expect("env filter init failed");

    if enabled {
        let config = app_logs_cfg.expect("app_logs_cfg missing but enabled=true");

        // 1. Initialize SQLite (async — need block_in_place since init_logger is sync).
        let db_result: Result<(), Box<dyn std::error::Error + Send + Sync>> =
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async { writer::init_db(&config.db_path).await })
            });
        db_result.map_err(loco_rs::Error::Any)?;

        // 2. Create channel + store sender globally.
        let (tx, rx) = flume::bounded(8192);
        layer::set_sender(tx);

        // 3. Start background writer + cleanup.
        writer::spawn_writer(rx, config);
        writer::spawn_cleanup(config.retention_days);

        // 4. Build subscriber with fmt + SQLite layer.
        let sqlite_layer = layer::SqliteTracingLayer::new(
            layer::get_sender().unwrap().clone(),
            config.level_filter(),
            config.capture_log_entries,
        );

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(sqlite_layer)
            .init();

        tracing::debug!(
            db_path = %config.db_path,
            capture_level = %config.capture_level,
            "[app-logs] module initialized"
        );
    } else {
        // Module disabled — use loco's standard logger setup.
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .init();
    }

    Ok(true) // We've taken over tracing init.
}
