//! `SQLite` WAL mode initializer.
//!
//! Enables Write-Ahead Logging on the main database so that concurrent readers
//! do not block writers and vice-versa. Without WAL, `SQLite` uses exclusive
//! write locks that cause "database is locked" errors when the QA pipeline
//! holds a long-lived connection while the frontend executor calls a write
//! endpoint (e.g. `POST /api/users`).

use async_trait::async_trait;
use loco_rs::app::{AppContext, Initializer};
use loco_rs::Result;

pub struct SqliteWalInitializer;

#[async_trait]
impl Initializer for SqliteWalInitializer {
    fn name(&self) -> String {
        "sqlite_wal".to_string()
    }

    async fn before_run(&self, ctx: &AppContext) -> Result<()> {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        let db = &ctx.db;

        let result = db
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA journal_mode = WAL",
            ))
            .await;

        match result {
            Ok(res) => {
                // PRAGMA journal_mode returns the new mode in the result rows.
                // We just log success; the PRAGMA is persistent for the file.
                tracing::info!(
                    rows_affected = res.rows_affected(),
                    "SQLite journal_mode set to WAL"
                );
            }
            Err(e) => {
                // Non-fatal: WAL is a performance optimisation, not a correctness
                // requirement. Log a warning but continue.
                tracing::warn!(error = %e, "Failed to set SQLite WAL mode (non-fatal)");
            }
        }

        // Also enable busy_timeout so short contention resolves gracefully
        // instead of immediately returning SQLITE_BUSY.
        let timeout_result = db
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA busy_timeout = 30000",
            ))
            .await;

        if let Err(e) = timeout_result {
            tracing::warn!(error = %e, "Failed to set SQLite busy_timeout (non-fatal)");
        }

        Ok(())
    }
}
