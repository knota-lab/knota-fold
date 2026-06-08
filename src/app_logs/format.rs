use std::collections::BTreeMap;
use std::fmt;

use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormattedFields;
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug, Default, Clone, Copy)]
pub struct BusinessLocationFormat;

impl<S, N> FormatEvent<S, N> for BusinessLocationFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let mut fields = ConsoleFieldVisitor::default();
        event.record(&mut fields);
        let ansi = writer.has_ansi_escapes();

        write_colored(&mut writer, ansi, AnsiColor::Dim, ChronoUtcNow)?;
        write!(writer, " ")?;
        write_level(&mut writer, ansi, *metadata.level())?;
        write!(writer, " ")?;
        write_spans(ctx, &mut writer, ansi)?;

        let file = fields
            .values
            .get("caller_file")
            .map(String::as_str)
            .or_else(|| metadata.file());
        let line = fields
            .values
            .get("caller_line")
            .cloned()
            .or_else(|| metadata.line().map(|line| line.to_string()));

        write_colored(&mut writer, ansi, AnsiColor::Cyan, metadata.target())?;
        write!(writer, ":")?;
        if let Some(file) = file {
            write!(writer, " ")?;
            write_colored(&mut writer, ansi, AnsiColor::Green, file)?;
            if let Some(line) = line.as_deref() {
                write_colored(
                    &mut writer,
                    ansi,
                    AnsiColor::Green,
                    format_args!(":{line}"),
                )?;
            }
            write!(writer, ":")?;
        }

        if let Some(message) = fields.message.as_deref() {
            write!(writer, " {message}")?;
        }

        for (key, value) in fields.values {
            if should_skip_console_field(&key) {
                continue;
            }
            write!(writer, " {key}={value}")?;
        }

        writeln!(writer)
    }
}

fn write_spans<S, N>(
    ctx: &FmtContext<'_, S, N>,
    writer: &mut Writer<'_>,
    ansi: bool,
) -> fmt::Result
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    ctx.visit_spans(|span| {
        write_colored(writer, ansi, AnsiColor::Purple, span.name())?;
        write!(writer, "{{")?;
        let extensions = span.extensions();
        if let Some(fields) = extensions.get::<FormattedFields<N>>() {
            write!(writer, "{fields}")?;
        }
        drop(extensions);
        write!(writer, "}}:")
    })?;
    Ok(())
}

#[derive(Clone, Copy)]
enum AnsiColor {
    Dim,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
    Cyan,
}

impl AnsiColor {
    const fn code(self) -> &'static str {
        match self {
            Self::Dim => "\x1b[2m",
            Self::Red => "\x1b[31m",
            Self::Yellow => "\x1b[33m",
            Self::Green => "\x1b[32m",
            Self::Blue => "\x1b[34m",
            Self::Purple => "\x1b[35m",
            Self::Cyan => "\x1b[36m",
        }
    }
}

fn write_colored(
    writer: &mut Writer<'_>,
    ansi: bool,
    color: AnsiColor,
    value: impl fmt::Display,
) -> fmt::Result {
    if ansi {
        write!(writer, "{}{value}\x1b[0m", color.code())
    } else {
        write!(writer, "{value}")
    }
}

fn write_level(
    writer: &mut Writer<'_>,
    ansi: bool,
    level: tracing::Level,
) -> fmt::Result {
    let color = match level {
        tracing::Level::ERROR => AnsiColor::Red,
        tracing::Level::WARN => AnsiColor::Yellow,
        tracing::Level::INFO => AnsiColor::Green,
        tracing::Level::DEBUG => AnsiColor::Blue,
        tracing::Level::TRACE => AnsiColor::Purple,
    };
    if ansi {
        write!(writer, "{}{:>5}\x1b[0m", color.code(), level)
    } else {
        write!(writer, "{level:>5}")
    }
}

fn should_skip_console_field(key: &str) -> bool {
    matches!(
        key,
        "caller_file" | "caller_line" | "caller_column" | "location"
    )
}

#[derive(Default)]
struct ConsoleFieldVisitor {
    message: Option<String>,
    values: BTreeMap<String, String>,
}

impl tracing::field::Visit for ConsoleFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.values
                .insert(field.name().to_string(), value.to_string());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let raw = format!("{value:?}");
        let value = raw.trim_matches('"').to_string();
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.values.insert(field.name().to_string(), value);
        }
    }
}

struct ChronoUtcNow;

impl fmt::Display for ChronoUtcNow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
        )
    }
}
