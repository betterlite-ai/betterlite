use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::messages::galileo_input_messages;
use super::metrics::{token_metrics_from_record, token_metrics_json};
use super::support::{has_value, record_i64, record_str_or};
use super::timestamps::normalize_created_at;

fn duration_metrics(record: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let duration_ns = record_i64(record, "latency_ms").saturating_mul(1_000_000);
    let mut metrics = IndexMap::new();
    metrics.insert(
        "duration_ns".to_owned(),
        Json::Number(serde_json::Number::from(duration_ns)),
    );
    metrics.extend(token_metrics_json(token_metrics_from_record(record)));
    metrics
}

fn span_output(record: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let content = record_str_or(record, "output_text", "");
    let mut output = IndexMap::new();
    output.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    output.insert("content".to_owned(), Json::Text(content));
    output
}

// Both envelopes open with type/id, nest trace_id/parent_id only for a span, then stamp node_type and created_at.
fn record_envelope_shell(
    record: &IndexMap<String, Json>,
    type_name: &str,
    id: &str,
    nested_id: Option<&str>,
) -> IndexMap<String, Json> {
    let mut envelope = IndexMap::new();
    envelope.insert("type".to_owned(), Json::Text(type_name.to_owned()));
    envelope.insert("id".to_owned(), Json::Text(id.to_owned()));
    if let Some(parent_trace_id) = nested_id {
        envelope.insert(
            "trace_id".to_owned(),
            Json::Text(parent_trace_id.to_owned()),
        );
        envelope.insert(
            "parent_id".to_owned(),
            Json::Text(parent_trace_id.to_owned()),
        );
    }
    envelope.insert(
        "name".to_owned(),
        Json::Text(record_str_or(record, "node_type", "litellm")),
    );
    envelope.insert(
        "created_at".to_owned(),
        Json::Text(normalize_created_at(&record_str_or(
            record,
            "created_at",
            "",
        ))),
    );
    envelope
}

fn status_code_or_ok(record: &IndexMap<String, Json>) -> Json {
    let Some(value) = record.get("status_code") else {
        return Json::Number(serde_json::Number::from(200_i64));
    };
    value.clone()
}

// litellm.integrations.galileo.GalileoObserve._record_to_v2_span; trace_id/span_id are the caller's injected UUIDs, since this layer generates none.
#[must_use]
pub fn record_to_v2_span(
    record: &IndexMap<String, Json>,
    trace_id: &str,
    span_id: &str,
) -> IndexMap<String, Json> {
    let mut span = record_envelope_shell(record, "llm", span_id, Some(trace_id));
    span.insert(
        "input".to_owned(),
        Json::List(galileo_input_messages(
            record.get("messages"),
            &record_str_or(record, "input_text", ""),
        )),
    );
    span.insert("output".to_owned(), Json::Object(span_output(record)));
    span.insert("status_code".to_owned(), status_code_or_ok(record));
    span.insert(
        "model".to_owned(),
        record.get("model").cloned().unwrap_or(Json::Null),
    );
    span.insert("metrics".to_owned(), Json::Object(duration_metrics(record)));
    if let Some(tags) = record.get("tags")
        && has_value(tags)
    {
        span.insert("tags".to_owned(), tags.clone());
    }
    span
}

// litellm.integrations.galileo.GalileoObserve._record_to_v2_trace.
#[must_use]
pub fn record_to_v2_trace(
    record: &IndexMap<String, Json>,
    trace_id: &str,
    span_id: &str,
) -> IndexMap<String, Json> {
    // record_to_v2_span computes status_code and metrics from this same record, so the trace envelope reads them back from the span it is about to nest instead of recomputing them.
    let span = record_to_v2_span(record, trace_id, span_id);
    let reused_fields: Vec<(String, Json)> = ["status_code", "metrics"]
        .into_iter()
        .filter_map(|key| span.get(key).map(|value| (key.to_owned(), value.clone())))
        .collect();

    record_envelope_shell(record, "trace", trace_id, None)
        .into_iter()
        .chain(reused_fields)
        .chain([
            (
                "input".to_owned(),
                Json::Text(record_str_or(record, "input_text", "")),
            ),
            (
                "output".to_owned(),
                Json::Text(record_str_or(record, "output_text", "")),
            ),
            ("spans".to_owned(), Json::List(vec![Json::Object(span)])),
        ])
        .collect()
}

// litellm.integrations.galileo.GalileoObserve._build_traces_payload; log_stream_id is config state owned by B3964, so it arrives as a parameter instead of being read from self.
#[must_use]
pub fn build_traces_payload(
    records: &[IndexMap<String, Json>],
    trace_span_ids: &[(String, String)],
    requested_log_stream_id: Option<&str>,
) -> IndexMap<String, Json> {
    let traces = records
        .iter()
        .zip(trace_span_ids)
        .map(|(record, (trace_id, span_id))| {
            Json::Object(record_to_v2_trace(record, trace_id, span_id))
        })
        .collect();

    let mut payload = IndexMap::new();
    payload.insert("traces".to_owned(), Json::List(traces));
    payload.insert(
        "logging_method".to_owned(),
        Json::Text("api_direct".to_owned()),
    );
    payload.insert("reliable".to_owned(), Json::Bool(false));
    payload.insert("is_complete".to_owned(), Json::Bool(true));
    if let Some(log_stream_id) = requested_log_stream_id
        && !log_stream_id.is_empty()
    {
        payload.insert(
            "log_stream_id".to_owned(),
            Json::Text(log_stream_id.to_owned()),
        );
    }
    payload
}
