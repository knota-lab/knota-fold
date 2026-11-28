/// SQL for creating the app-logs tables and indexes.
/// Executed with `CREATE TABLE IF NOT EXISTS` at startup — no migration needed.
pub const CREATE_TABLES: &str = r"
CREATE TABLE IF NOT EXISTS request_logs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id    TEXT    NOT NULL,
    request_id  TEXT    NOT NULL,
    timestamp   INTEGER NOT NULL,
    method      TEXT    NOT NULL,
    path        TEXT    NOT NULL,
    route       TEXT,
    status_code INTEGER,
    duration_ms INTEGER,
    user_id     TEXT,
    tenant_code TEXT,
    ip_address  TEXT,
    error       TEXT
);

CREATE TABLE IF NOT EXISTS trace_spans (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id        TEXT    NOT NULL,
    span_id         TEXT    NOT NULL,
    parent_span_id  TEXT,
    span_name       TEXT    NOT NULL,
    start_time      INTEGER NOT NULL,
    duration_ms     INTEGER,
    span_type       TEXT,
    fields_json     TEXT
);

CREATE TABLE IF NOT EXISTS log_entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id    TEXT    NOT NULL,
    span_id     TEXT,
    timestamp   INTEGER NOT NULL,
    level       TEXT    NOT NULL,
    target      TEXT,
    message     TEXT,
    fields_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_req_logs_trace_id    ON request_logs(trace_id);
CREATE INDEX IF NOT EXISTS idx_req_logs_timestamp    ON request_logs(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_req_logs_route        ON request_logs(route);
CREATE INDEX IF NOT EXISTS idx_req_logs_path         ON request_logs(path);
CREATE INDEX IF NOT EXISTS idx_req_logs_user_id      ON request_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_req_logs_tenant_code  ON request_logs(tenant_code);
CREATE INDEX IF NOT EXISTS idx_spans_trace_id        ON trace_spans(trace_id);
CREATE INDEX IF NOT EXISTS idx_spans_parent_span_id  ON trace_spans(parent_span_id);
CREATE INDEX IF NOT EXISTS idx_log_entries_trace_id  ON log_entries(trace_id);
";
