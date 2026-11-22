use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_headers::SpanContext;
use crate::api::request_metadata_safety::metadata::MetadataField;

pub const LITELLM_TRACE_ID: &str = "litellm_trace_id";
pub const TRACE_ID_KEY: &str = "trace_id";
pub const TRACEPARENT: &str = "traceparent";
const METADATA_KEY: &str = "metadata";
const EXTRA_HEADERS: &str = "extra_headers";
const SESSION_ID_ENTRY: &str = "session.id";
// version, trace-id, parent-id and flags of a W3C traceparent, each a fixed run of hex digits
const TRACEPARENT_FIELD_LENGTHS: [usize; 4] = [2, 32, 16, 2];

#[must_use]
pub fn trace_id_from_traceparent(traceparent: &str) -> Option<String> {
    let fields: Vec<&str> = traceparent.trim().split('-').collect();
    let shaped = fields.len() == TRACEPARENT_FIELD_LENGTHS.len()
        && fields
            .iter()
            .zip(TRACEPARENT_FIELD_LENGTHS)
            .all(|(field, length)| {
                field.len() == length && field.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
    if !shaped {
        return None;
    }
    let trace_id = fields.get(1)?.to_ascii_lowercase();
    // an all-zero trace-id is invalid, as the OpenTelemetry SDK treats it
    trace_id
        .bytes()
        .any(|byte| byte != b'0')
        .then_some(trace_id)
}

#[must_use]
pub fn session_id_from_baggage(baggage: &str) -> Option<String> {
    baggage.split(',').find_map(|pair| {
        let entry = pair.trim();
        let (key, value) = entry.split_once('=').unwrap_or((entry, ""));
        let session_id = value.trim();
        (key.trim() == SESSION_ID_ENTRY && !session_id.is_empty()).then(|| session_id.to_owned())
    })
}

// the caller's own trace and an id already in either metadata bucket outrank the proxy's span
pub fn add_otel_trace_id(
    data: &mut RequestData,
    slot: MetadataField,
    parent_span: Option<SpanContext>,
) {
    let has_trace_id = |bucket: Option<&Json>| {
        matches!(bucket, Some(Json::Object(fields))
            if fields.get(TRACE_ID_KEY).is_some_and(|trace_id| *trace_id != Json::Null))
    };
    if data
        .get(LITELLM_TRACE_ID)
        .is_some_and(|trace_id| *trace_id != Json::Null)
        || has_trace_id(data.get(slot.as_str()))
        || has_trace_id(data.get(METADATA_KEY))
    {
        return;
    }
    let Some(span) = parent_span.filter(|span| span.is_valid) else {
        return;
    };
    let trace_id = format!("{:032x}", span.trace_id);
    data.insert(LITELLM_TRACE_ID.to_owned(), Json::Text(trace_id.clone()));
    if let Some(Json::Object(bucket)) = data.get_mut(slot.as_str()) {
        bucket.insert(TRACE_ID_KEY.to_owned(), Json::Text(trace_id));
    }
}

// forward_traceparent_to_llm_provider sends the caller's traceparent on, never over one the request already set
pub fn forward_traceparent(
    data: &mut RequestData,
    headers: &IndexMap<String, String>,
    enabled: bool,
) {
    if !enabled {
        return;
    }
    let Some((_name, traceparent)) = headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(TRACEPARENT))
    else {
        return;
    };
    let extra_headers = data.entry(EXTRA_HEADERS.to_owned()).or_insert(Json::Null);
    if *extra_headers == Json::Null {
        *extra_headers = Json::Object(IndexMap::new());
    }
    if let Json::Object(fields) = extra_headers {
        fields
            .entry(TRACEPARENT.to_owned())
            .or_insert_with(|| Json::Text(traceparent.clone()));
    }
}
