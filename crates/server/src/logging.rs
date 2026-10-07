//! Log output. `LIFEAGENT_LOG_FORMAT=json` writes one JSON object per line in Cloud Logging's
//! structured format (`severity`, `message`, `logging.googleapis.com/sourceLocation`), with the
//! event's fields and the fields of every enclosing span as top-level keys. Cloud Logging
//! parses those into `jsonPayload`, so lines can be filtered by e.g. `jsonPayload.user_id` or
//! `jsonPayload.model`. Anything else (the default) writes the usual human-readable text.

use std::fmt;

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Keys the formatter writes itself; fields with these names are prefixed with `field_` so
/// they can't overwrite them.
const RESERVED: &[&str] = &[
    "severity",
    "message",
    "target",
    "span",
    "logging.googleapis.com/sourceLocation",
];

/// Installs the global subscriber. `RUST_LOG` filters (default `info`).
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = std::env::var("LIFEAGENT_LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json"));
    if json {
        tracing_subscriber::registry()
            .with(filter)
            .with(SpanFieldsLayer)
            .with(tracing_subscriber::fmt::layer().event_format(CloudLoggingFormat))
            .init();
    } else {
        // Colors only on a terminal.
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
            .init();
    }
}

fn severity(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARNING",
        Level::INFO => "INFO",
        Level::DEBUG | Level::TRACE => "DEBUG",
    }
}

/// Collects fields as typed JSON values.
struct JsonVisitor<'a>(&'a mut Map<String, Value>);

impl JsonVisitor<'_> {
    fn insert(&mut self, field: &Field, value: Value) {
        let name = field.name();
        let key = if RESERVED.contains(&name) && name != "message" {
            format!("field_{name}")
        } else {
            name.to_string()
        };
        self.0.insert(key, value);
    }
}

impl Visit for JsonVisitor<'_> {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, value.into());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field, value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, value.into());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.insert(field, value.to_string().into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.insert(field, format!("{value:?}").into());
    }
}

/// A span's fields, stored in its extensions by `SpanFieldsLayer`.
struct SpanFields(Map<String, Value>);

/// Keeps each span's fields as JSON so the formatter can copy them onto every event inside it.
struct SpanFieldsLayer;

impl<S> Layer<S> for SpanFieldsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut fields = Map::new();
        attrs.record(&mut JsonVisitor(&mut fields));
        span.extensions_mut().insert(SpanFields(fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        if let Some(SpanFields(fields)) = extensions.get_mut::<SpanFields>() {
            values.record(&mut JsonVisitor(fields));
        }
    }
}

/// One Cloud Logging JSON line per event.
struct CloudLoggingFormat;

impl<S, N> FormatEvent<S, N> for CloudLoggingFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'w> FormatFields<'w> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let mut entry = Map::new();
        // Outermost span first, so an inner span's field wins over an outer one's.
        let mut span_names = Vec::new();
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                span_names.push(span.name());
                if let Some(SpanFields(fields)) = span.extensions().get::<SpanFields>() {
                    entry.extend(fields.clone());
                }
            }
        }
        event.record(&mut JsonVisitor(&mut entry));
        entry.insert("severity".into(), severity(meta.level()).into());
        entry.insert("target".into(), meta.target().into());
        if !span_names.is_empty() {
            entry.insert("span".into(), span_names.join(":").into());
        }
        if let (Some(file), Some(line)) = (meta.file(), meta.line()) {
            entry.insert(
                "logging.googleapis.com/sourceLocation".into(),
                serde_json::json!({ "file": file, "line": line.to_string() }),
            );
        }
        let line = serde_json::to_string(&entry).map_err(|_| fmt::Error)?;
        writeln!(writer, "{line}")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture(f: impl FnOnce()) -> Vec<Value> {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::registry().with(SpanFieldsLayer).with(
            tracing_subscriber::fmt::layer()
                .event_format(CloudLoggingFormat)
                .with_writer(move || writer.clone()),
        );
        tracing::subscriber::with_default(subscriber, f);
        let out = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        out.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn writes_cloud_logging_fields_with_typed_values() {
        let lines = capture(|| {
            tracing::warn!(tokens = 12u64, ok = true, model = %"m", "llm call");
        });
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line["severity"], "WARNING");
        assert_eq!(line["message"], "llm call");
        assert_eq!(line["tokens"], 12);
        assert_eq!(line["ok"], true);
        assert_eq!(line["model"], "m");
        assert!(line["logging.googleapis.com/sourceLocation"]["file"]
            .as_str()
            .unwrap()
            .ends_with("logging.rs"));
    }

    #[test]
    fn copies_span_fields_onto_events_inner_span_winning() {
        let lines = capture(|| {
            let outer = tracing::info_span!("command", user_id = "u1", command = "x");
            let _outer = outer.enter();
            let inner = tracing::info_span!("inner", command = "y", late = tracing::field::Empty);
            inner.record("late", 3);
            let _inner = inner.enter();
            tracing::error!("boom");
        });
        let line = &lines[0];
        assert_eq!(line["severity"], "ERROR");
        assert_eq!(line["user_id"], "u1");
        assert_eq!(line["command"], "y");
        assert_eq!(line["late"], 3);
        assert_eq!(line["span"], "command:inner");
    }

    #[test]
    fn reserved_field_names_cannot_overwrite_severity() {
        let lines = capture(|| tracing::info!(severity = "fake", "hi"));
        assert_eq!(lines[0]["severity"], "INFO");
        assert_eq!(lines[0]["field_severity"], "fake");
    }
}
