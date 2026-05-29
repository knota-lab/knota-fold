use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use flume::Sender;
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

// ── LogEntry: enum payload ──────────────────────────────────────

#[derive(Debug)]
pub struct LogEntry {
    pub trace_id: String,
    pub timestamp: i64,
    pub payload: EntryPayload,
}

#[derive(Debug)]
pub enum EntryPayload {
    /// Sent by TracingLayer on request completion → request_logs table.
    RequestSummary {
        request_id: String,
        method: String,
        path: String,
        route: Option<String>,
        status_code: i64,
        duration_ms: i64,
        user_id: Option<String>,
        tenant_code: Option<String>,
        ip_address: Option<String>,
        error: Option<String>,
    },
    /// Sent on span close → trace_spans table.
    SpanClose {
        span_id: String,
        parent_span_id: Option<String>,
        span_name: String,
        span_type: String,
        start_time: i64,
        duration_ms: i64,
        fields_json: Option<String>,
    },
    /// Sent on tracing event → log_entries table.
    LogLine {
        span_id: Option<String>,
        level: String,
        target: String,
        message: String,
        fields_json: Option<String>,
    },
}

impl LogEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn request_summary(
        trace_id: &str,
        request_id: &str,
        method: &str,
        path: &str,
        route: Option<&str>,
        status_code: u16,
        duration_ms: u64,
        user_id: Option<&str>,
        tenant_code: Option<&str>,
        ip_address: Option<&str>,
        error: Option<&str>,
    ) -> Self {
        Self {
            trace_id: trace_id.to_string(),
            timestamp: unix_ms(),
            payload: EntryPayload::RequestSummary {
                request_id: request_id.to_string(),
                method: method.to_string(),
                path: path.to_string(),
                route: route.map(String::from),
                status_code: i64::from(status_code),
                duration_ms: i64::try_from(duration_ms).unwrap_or(-1),
                user_id: user_id.map(String::from),
                tenant_code: tenant_code.map(String::from),
                ip_address: ip_address.map(String::from),
                error: error.map(String::from),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn span_close(
        trace_id: String,
        span_id: String,
        parent_span_id: Option<String>,
        span_name: String,
        span_type: String,
        start_time: i64,
        duration_ms: i64,
        fields_json: Option<String>,
    ) -> Self {
        Self {
            trace_id,
            timestamp: unix_ms(),
            payload: EntryPayload::SpanClose {
                span_id,
                parent_span_id,
                span_name,
                span_type,
                start_time,
                duration_ms,
                fields_json,
            },
        }
    }

    pub fn log_line(
        trace_id: String,
        span_id: Option<String>,
        level: String,
        target: String,
        message: String,
        fields_json: Option<String>,
    ) -> Self {
        Self {
            trace_id,
            timestamp: unix_ms(),
            payload: EntryPayload::LogLine {
                span_id,
                level,
                target,
                message,
                fields_json,
            },
        }
    }
}

// ── Global sender ───────────────────────────────────────────────

pub type LogSender = Sender<LogEntry>;

static SENDER: OnceLock<LogSender> = OnceLock::new();

/// Store the channel sender globally. Called from `init_logger`.
pub fn set_sender(sender: LogSender) {
    SENDER.set(sender).ok();
}

/// Retrieve the global sender. Returns `None` if module is disabled.
pub fn get_sender() -> Option<&'static LogSender> {
    SENDER.get()
}

// ── Span tracking state ─────────────────────────────────────────

/// Metadata cached when a span is created, consumed on span close.
struct ActiveSpan {
    trace_id: String,
    span_name: String,
    span_type: String,
    parent_span_id: Option<String>,
    start_time: i64,
    api_key_id: Option<String>,
    auth_type: Option<String>,
    /// JSON-serializable map of all recorded fields.
    fields_json: Option<String>,
}

/// Extension stored in each span so child spans/events can find trace_id O(1).
#[derive(Clone)]
pub struct TracedId(pub String);

// ── SqliteTracingLayer ──────────────────────────────────────────

pub struct SqliteTracingLayer {
    sender: LogSender,
    /// Active spans: tracing internal Id → ActiveSpan metadata.
    active_spans: Mutex<HashMap<u64, ActiveSpan>>,
    /// Minimum log level to capture for LogLine entries.
    capture_level: tracing::Level,
    /// Whether to capture LogLine entries at all.
    capture_log_entries: bool,
}

impl SqliteTracingLayer {
    pub fn new(
        sender: LogSender,
        capture_level: tracing::Level,
        capture_log_entries: bool,
    ) -> Self {
        Self {
            sender,
            active_spans: Mutex::new(HashMap::new()),
            capture_level,
            capture_log_entries,
        }
    }
}

impl<S> Layer<S> for SqliteTracingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &Id,
        ctx: Context<'_, S>,
    ) {
        let span_id_u64 = id.into_u64();

        // Try to extract trace_id and parent_span_id from this span's fields.
        let mut extractor = SpanFieldExtractor::default();
        attrs.values().record(&mut extractor);
        let trace_id = extractor.trace_id.unwrap_or_else(|| {
            // Not on this span — walk parent chain.
            extract_trace_id_from_context(&ctx, id)
        });

        // Inject TracedId into span extensions so children can find it.
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(TracedId(trace_id.clone()));
        }

        // Use explicit parent_span_id from span fields (e.g., worker args)
        // over the tracing parent. Workers run in separate async contexts
        // where the HTTP request span is not a tracing parent.
        let parent_span_id = extractor.parent_span_id.or_else(|| {
            ctx.span(id)
                .and_then(|s| s.parent().map(|p| p.id().into_u64().to_string()))
        });

        let span_name = attrs.metadata().name().to_string();
        let span_type = classify_span(&span_name);

        let fields_json = if extractor.extra.is_empty() {
            None
        } else {
            serde_json::to_string(&extractor.extra).ok()
        };

        let mut spans = self.active_spans.lock().unwrap();
        spans.insert(
            span_id_u64,
            ActiveSpan {
                trace_id,
                span_name,
                span_type,
                parent_span_id,
                start_time: unix_ms(),
                api_key_id: extractor.api_key_id,
                auth_type: extractor.auth_type,
                fields_json,
            },
        );
    }

    fn on_record(
        &self,
        id: &Id,
        values: &tracing::span::Record<'_>,
        _ctx: Context<'_, S>,
    ) {
        let mut extractor = SpanFieldExtractor::default();
        values.record(&mut extractor);

        let mut spans = self.active_spans.lock().unwrap();
        if let Some(active) = spans.get_mut(&id.into_u64()) {
            if extractor.api_key_id.is_some() {
                active.api_key_id = extractor.api_key_id;
            }
            if extractor.auth_type.is_some() {
                active.auth_type = extractor.auth_type;
            }
            // Merge extra fields from record into existing fields_json
            if !extractor.extra.is_empty() {
                let mut existing: serde_json::Map<String, serde_json::Value> = active
                    .fields_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                existing.extend(extractor.extra);
                active.fields_json = serde_json::to_string(&existing).ok();
            }
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        let span_id_u64 = id.into_u64();
        let active = {
            let mut spans = self.active_spans.lock().unwrap();
            spans.remove(&span_id_u64)
        };

        if let Some(active) = active {
            let duration = unix_ms() - active.start_time;
            try_send_log(
                &self.sender,
                LogEntry::span_close(
                    active.trace_id,
                    span_id_u64.to_string(),
                    active.parent_span_id,
                    active.span_name,
                    active.span_type,
                    active.start_time,
                    duration,
                    active.fields_json,
                ),
            );
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if !self.capture_log_entries {
            return;
        }

        // Check level filter.
        if *event.metadata().level() > self.capture_level {
            return;
        }

        let current_span = ctx.lookup_current();
        let trace_id = current_span.as_ref().map_or_else(
            || "untraced".to_string(),
            |s| {
                s.extensions()
                    .get::<TracedId>()
                    .map_or_else(|| "untraced".to_string(), |tid| tid.0.clone())
            },
        );
        let span_id = current_span.as_ref().map(|s| s.id().into_u64().to_string());

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        // Inject source location from event metadata (tracing natively tracks file/line)
        if let (Some(file), Some(line)) =
            (event.metadata().file(), event.metadata().line())
        {
            visitor.fields.insert("file".to_string(), file.to_string());
            visitor.fields.insert("line".to_string(), line.to_string());
        }

        let fields_json = if visitor.fields.is_empty() {
            None
        } else {
            serde_json::to_string(&visitor.fields).ok()
        };

        try_send_log(
            &self.sender,
            LogEntry::log_line(
                trace_id,
                span_id,
                event.metadata().level().to_string(),
                event.metadata().target().to_string(),
                visitor.message.unwrap_or_default(),
                fields_json,
            ),
        );
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Walk the span chain upward to find a TracedId extension.
fn extract_trace_id_from_context<S>(ctx: &Context<'_, S>, id: &Id) -> String
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let mut current = ctx.span(id);
    while let Some(span) = current {
        if let Some(tid) = span.extensions().get::<TracedId>() {
            return tid.0.clone();
        }
        current = span.parent();
    }
    "untraced".to_string()
}

/// Classify a span name into a type for frontend filtering/coloring.
fn classify_span(name: &str) -> String {
    if name.starts_with("http") {
        "http".to_string()
    } else if name.contains("db") || name.contains("query") || name.contains("sql") {
        "db".to_string()
    } else if name.contains("cache") {
        "cache".to_string()
    } else if name.contains("task") || name.contains("job") || name.contains("worker") {
        "task".to_string()
    } else {
        "service".to_string()
    }
}

/// Extracts `trace_id`, `parent_span_id` and all other fields from span attributes.
#[derive(Default)]
struct SpanFieldExtractor {
    trace_id: Option<String>,
    parent_span_id: Option<String>,
    api_key_id: Option<String>,
    auth_type: Option<String>,
    /// All other fields collected as key-value pairs for fields_json persistence.
    extra: serde_json::Map<String, serde_json::Value>,
}

impl tracing::field::Visit for SpanFieldExtractor {
    fn record_debug(
        &mut self,
        field: &tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        let raw = format!("{value:?}");
        match field.name() {
            "trace_id" => {
                self.trace_id = Some(raw.trim_matches('"').to_string());
            }
            "parent_span_id" => {
                self.parent_span_id = Some(raw.trim_matches('"').to_string());
            }
            "api_key_id" => {
                self.api_key_id = Some(raw.trim_matches('"').to_string());
            }
            "auth_type" => {
                self.auth_type = Some(raw.trim_matches('"').to_string());
            }
            _ => {
                self.extra.insert(
                    field.name().to_string(),
                    serde_json::Value::String(raw.trim_matches('"').to_string()),
                );
            }
        }
    }
}

/// Extracts the formatted message AND all structured fields from an event.
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: std::collections::BTreeMap<String, String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(
        &mut self,
        field: &tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        let raw = format!("{value:?}");
        let v = raw.trim_matches('"').to_string();
        if field.name() == "message" {
            self.message = Some(v);
        } else {
            self.fields.insert(field.name().to_string(), v);
        }
    }
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// ── Dropped-log observability ───────────────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};

static DROPPED_COUNT: AtomicU64 = AtomicU64::new(0);

/// Non-blocking send with drop counting.
pub fn try_send_log(sender: &LogSender, entry: LogEntry) {
    if sender.try_send(entry).is_err() {
        let prev = DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
        if prev > 0 && prev.is_multiple_of(1000) {
            tracing::warn!(dropped = prev + 1, "[app-logs] logs dropped (channel full)");
        }
    }
}

pub fn dropped_count() -> u64 {
    DROPPED_COUNT.load(Ordering::Relaxed)
}
