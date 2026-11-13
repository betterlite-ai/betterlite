use betterlite_models::decisions::Json;

use super::Record;

fn list_items(value: Option<&Json>) -> &[Json] {
    match value {
        Some(Json::List(items)) => items,
        _ => &[],
    }
}

fn span_missing_fields(prefix: &str, span: &Record, trace_id: Option<&Json>) -> Vec<String> {
    let mut missing = Vec::new();
    for field in ["id", "trace_id", "parent_id"] {
        if !span.contains_key(field) {
            missing.push(format!("{prefix}.{field}"));
        }
    }
    let has_trace_id = matches!(trace_id, Some(Json::Text(id)) if !id.is_empty());
    if has_trace_id && span.get("trace_id") != trace_id {
        missing.push(format!("{prefix}.trace_id mismatch"));
    }
    missing
}

fn trace_missing_fields(trace_index: usize, trace: &Record) -> Vec<String> {
    let mut missing = Vec::new();
    for field in ["id", "type", "spans"] {
        if !trace.contains_key(field) {
            missing.push(format!("traces[{trace_index}].{field}"));
        }
    }
    for (span_index, item) in list_items(trace.get("spans")).iter().enumerate() {
        if let Json::Object(span) = item {
            let prefix = format!("traces[{trace_index}].spans[{span_index}]");
            missing.extend(span_missing_fields(&prefix, span, trace.get("id")));
        }
    }
    missing
}

// A trace missing its id can only be named by position for the operator.
#[must_use]
pub fn v2_payload_missing_fields(payload: &Record) -> Vec<String> {
    let traces = list_items(payload.get("traces"));
    let mut missing = Vec::new();
    if traces.is_empty() {
        missing.push("traces".to_owned());
    }
    for (trace_index, item) in traces.iter().enumerate() {
        if let Json::Object(trace) = item {
            missing.extend(trace_missing_fields(trace_index, trace));
        }
    }
    missing
}

pub fn log_flush_payload(use_v2_api: bool, url: &str, payload: &Record) {
    let trace_count = list_items(payload.get("traces")).len();
    tracing::debug!("Galileo Logger flush URL: {url} trace_count={trace_count}");
    if use_v2_api && url.contains("/ingest/traces/") {
        let missing = v2_payload_missing_fields(payload);
        if !missing.is_empty() {
            tracing::debug!(
                "Galileo Logger: ingest /traces payload validation issues: {missing:?}"
            );
        }
    }
}
