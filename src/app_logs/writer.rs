use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use sqlx::SqlitePool;

use super::config::AppLogsConfig;
use super::layer::{EntryPayload, LogEntry};

static LOG_DB: OnceLock<SqlitePool> = OnceLock::new();

/// Get the app-logs SQLite pool. Returns None if module is disabled.
pub fn log_db() -> Option<&'static SqlitePool> {
    LOG_DB.get()
}

/// Create data directory, connect to SQLite, set WAL pragmas, create tables.
pub async fn init_db(
    db_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let pool = SqlitePool::connect(&format!("sqlite://{db_path}?mode=rwc")).await?;

    // WAL mode for concurrent read/write.
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA synchronous = NORMAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA cache_size = -8000")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA temp_store = MEMORY")
        .execute(&pool)
        .await?;

    // Create tables + indexes.
    sqlx::query(super::schema::CREATE_TABLES)
        .execute(&pool)
        .await?;

    LOG_DB.set(pool).ok();
    Ok(())
}

/// Spawn the background batch-writer task.
pub fn spawn_writer(rx: flume::Receiver<LogEntry>, config: &AppLogsConfig) {
    let batch_size = config.batch_size;
    let interval_ms = config.batch_interval_ms;
    let capture_entries = config.capture_log_entries;

    tokio::spawn(async move {
        let mut buf: Vec<LogEntry> = Vec::with_capacity(batch_size);
        let timeout = Duration::from_millis(interval_ms);

        loop {
            // Wait for first entry or timeout.
            match tokio::time::timeout(timeout, rx.recv_async()).await {
                Ok(Ok(entry)) => buf.push(entry),
                Ok(Err(_)) => break, // channel closed → shutdown
                Err(_) => {}         // timeout → flush below
            }

            // Drain as many as we can up to batch_size.
            while buf.len() < batch_size {
                match rx.try_recv() {
                    Ok(e) => buf.push(e),
                    Err(_) => break,
                }
            }

            if !buf.is_empty() {
                match flush_batch(&buf, capture_entries).await {
                    Ok(()) => {
                        CONSECUTIVE_FAILS.store(0, Ordering::Relaxed);
                    }
                    Err(e) => {
                        let fails = CONSECUTIVE_FAILS.fetch_add(1, Ordering::Relaxed);
                        if fails < 3 {
                            eprintln!("[app-logs] write error: {e}");
                        } else if fails.is_multiple_of(10) {
                            tracing::error!(
                                fails,
                                error = %e,
                                "[app-logs] consecutive write failures"
                            );
                        }
                    }
                }
                buf.clear();
            }
        }
    });
}

static CONSECUTIVE_FAILS: AtomicU64 = AtomicU64::new(0);

async fn flush_batch(
    entries: &[LogEntry],
    capture_entries: bool,
) -> Result<(), sqlx::Error> {
    let db = log_db().expect("flush_batch called but DB not initialized");
    let mut tx = db.begin().await?;

    for entry in entries {
        match &entry.payload {
            EntryPayload::RequestSummary {
                request_id,
                method,
                path,
                route,
                status_code,
                duration_ms,
                user_id,
                tenant_code,
                ip_address,
                error,
            } => {
                sqlx::query(
                    "INSERT INTO request_logs \
                     (trace_id, request_id, timestamp, method, path, route, \
                      status_code, duration_ms, user_id, tenant_code, ip_address, error) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&entry.trace_id)
                .bind(request_id)
                .bind(entry.timestamp)
                .bind(method)
                .bind(path)
                .bind(route)
                .bind(status_code)
                .bind(duration_ms)
                .bind(user_id)
                .bind(tenant_code)
                .bind(ip_address)
                .bind(error)
                .execute(&mut *tx)
                .await?;
            }
            EntryPayload::SpanClose {
                span_id,
                parent_span_id,
                span_name,
                span_type,
                start_time,
                duration_ms,
                fields_json,
            } => {
                sqlx::query(
                    "INSERT INTO trace_spans \
                     (trace_id, span_id, parent_span_id, span_name, \
                      start_time, duration_ms, span_type, fields_json) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&entry.trace_id)
                .bind(span_id)
                .bind(parent_span_id)
                .bind(span_name)
                .bind(start_time)
                .bind(duration_ms)
                .bind(span_type)
                .bind(fields_json)
                .execute(&mut *tx)
                .await?;
            }
            EntryPayload::LogLine {
                span_id,
                level,
                target,
                message,
                fields_json,
            } if capture_entries => {
                sqlx::query(
                    "INSERT INTO log_entries \
                     (trace_id, span_id, timestamp, level, target, message, fields_json) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&entry.trace_id)
                .bind(span_id)
                .bind(entry.timestamp)
                .bind(level)
                .bind(target)
                .bind(message)
                .bind(fields_json)
                .execute(&mut *tx)
                .await?;
            }
            EntryPayload::LogLine { .. } => {} // capture disabled, skip
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Spawn periodic cleanup of expired logs.
pub fn spawn_cleanup(retention_days: u32) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let Some(db) = log_db() else { continue };

            let cutoff = unix_ms() - (i64::from(retention_days) * 86_400_000);

            for table in ["request_logs", "log_entries"] {
                let _ = sqlx::query(&format!("DELETE FROM {table} WHERE timestamp < ?"))
                    .bind(cutoff)
                    .execute(db)
                    .await;
            }
            // trace_spans uses start_time, not timestamp.
            let _ = sqlx::query("DELETE FROM trace_spans WHERE start_time < ?")
                .bind(cutoff)
                .execute(db)
                .await;

            // PASSIVE checkpoint — doesn't block readers/writers.
            let _ = sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                .execute(db)
                .await;
        }
    });
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
