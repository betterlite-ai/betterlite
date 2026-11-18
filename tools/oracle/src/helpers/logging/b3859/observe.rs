use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use opentelemetry::KeyValue;
use opentelemetry::trace::{SpanId, Status};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use serde_json::{Value, json};

// the same shape the Python helper answers, in the same order the case compares
pub(super) fn observed(choice: &str, exporter: &InMemorySpanExporter) -> Result<Value, String> {
    let spans = exporter
        .get_finished_spans()
        .map_err(|error| error.to_string())?;
    let names: Vec<(SpanId, String)> = spans
        .iter()
        .map(|span| (span.span_context.span_id(), span.name.to_string()))
        .collect();
    let mut lines: Vec<(i64, i64, String, String, Value)> =
        spans.iter().map(|span| line(span, &names)).collect();
    lines.sort_by(|left, right| {
        (&left.0, &left.1, &left.2, &left.3).cmp(&(&right.0, &right.1, &right.2, &right.3))
    });
    let observed_spans: Vec<Value> = lines.into_iter().map(|row| row.4).collect();
    Ok(json!({"choice": choice, "spans": observed_spans}))
}

fn line(span: &SpanData, names: &[(SpanId, String)]) -> (i64, i64, String, String, Value) {
    let name = span.name.to_string();
    let parent = names
        .iter()
        .find(|(id, _)| *id == span.parent_span_id)
        .map_or_else(String::new, |(_, known)| known.clone());
    let start = system_ns(span.start_time);
    let end = system_ns(span.end_time);
    let value = json!({
        "name": name,
        "parent": parent,
        "kind": format!("{:?}", span.span_kind).to_uppercase(),
        "status": status_name(&span.status),
        "start": start.to_string(),
        "end": end.to_string(),
        "attributes": attributes_json(&span.attributes),
    });
    (start, end, name, parent, value)
}

fn system_ns(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |duration| {
        i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
    })
}

const fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Unset => "UNSET",
        Status::Ok => "OK",
        Status::Error { .. } => "ERROR",
    }
}

fn attributes_json(attributes: &[KeyValue]) -> BTreeMap<String, String> {
    attributes
        .iter()
        .map(|pair| (pair.key.as_str().to_owned(), value_text(&pair.value)))
        .collect()
}

fn value_text(value: &opentelemetry::Value) -> String {
    match value {
        opentelemetry::Value::Bool(flag) => flag.to_string(),
        opentelemetry::Value::I64(number) => number.to_string(),
        opentelemetry::Value::F64(number) => format!("{number:?}"),
        opentelemetry::Value::String(text) => json_text(text.as_str()),
        opentelemetry::Value::Array(_) | _ => String::new(),
    }
}

fn json_text(text: &str) -> String {
    if !(text.starts_with('{') || text.starts_with('[')) {
        return text.to_owned();
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return text.to_owned();
    };
    serde_json::to_string(&value).unwrap_or_else(|_| text.to_owned())
}
