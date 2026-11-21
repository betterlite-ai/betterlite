use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_headers::trace_context::{
    LITELLM_TRACE_ID, TRACE_ID_KEY, TRACEPARENT, session_id_from_baggage, trace_id_from_traceparent,
};
use crate::api::request_headers::{ForwardedHeaders, HeaderValue};
// ast-grep-ignore: no-long-names - B1105 defines this name
use crate::api::request_metadata_safety::identity::anthropic_session_id_from_metadata;
use crate::api::request_metadata_safety::identity::get_chain_id_from_headers;
use crate::api::request_metadata_safety::metadata::MetadataField;

const SPEND_LOGS_HEADER: &str = "x-litellm-spend-logs-metadata";
const AGENT_ID_HEADER: &str = "x-litellm-agent-id";
const BAGGAGE: &str = "baggage";
const LITELLM_SESSION_ID: &str = "litellm_session_id";
const SPEND_LOGS_KEY: &str = "spend_logs_metadata";
const AGENT_ID_KEY: &str = "agent_id";
const SESSION_ID_KEY: &str = "session_id";
const USER_ID_KEY: &str = "user_id";
const METADATA_KEY: &str = "metadata";

type Metadata = IndexMap<String, Json>;

// LitellmMetadataFromRequestHeaders: the spend-log, agent, trace and session ids a caller can send as headers
pub fn add_header_metadata(
    headers: &ForwardedHeaders,
    data: &mut RequestData,
    slot: MetadataField,
    limits: ApiLimits,
) {
    let mut from_headers = Metadata::new();
    if let Some(spend_logs) = headers
        .get(SPEND_LOGS_HEADER)
        .and_then(|value| spend_logs_metadata(value.expose(), limits))
    {
        from_headers.insert(SPEND_LOGS_KEY.to_owned(), spend_logs);
    }
    if let Some(agent_id) = headers
        .get(AGENT_ID_HEADER)
        .map(HeaderValue::expose)
        .filter(|agent_id| !agent_id.is_empty())
    {
        from_headers.insert(AGENT_ID_KEY.to_owned(), Json::Text(agent_id.to_owned()));
        tracing::debug!("Extracted agent_id from header: {agent_id}");
    }
    let plain: IndexMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose().to_owned()))
        .collect();
    match get_chain_id_from_headers(&plain).filter(|chain_id| !chain_id.is_empty()) {
        Some(chain_id) => {
            for key in [TRACE_ID_KEY, SESSION_ID_KEY] {
                from_headers.insert(key.to_owned(), Json::Text(chain_id.clone()));
            }
            for key in [LITELLM_SESSION_ID, LITELLM_TRACE_ID] {
                data.insert(key.to_owned(), Json::Text(chain_id.clone()));
            }
            tracing::debug!("Extracted chain_id from header (trace-id/session-id): {chain_id}");
        }
        None => add_anthropic_session(data, &mut from_headers),
    }
    add_w3c_fallback(&plain, data, &mut from_headers);
    if let Some(Json::Object(bucket)) = data.get_mut(slot.as_str()) {
        bucket.extend(from_headers);
    }
}

// LiteLLM's safe_json_loads turns text that is not JSON into None, so such a header is ignored
fn spend_logs_metadata(text: &str, limits: ApiLimits) -> Option<Json> {
    if text.len() > limits.max_json_bytes {
        tracing::warn!(
            header = SPEND_LOGS_HEADER,
            "the header is larger than the JSON limit and is ignored"
        );
        return None;
    }
    serde_json::from_str::<Json>(text)
        .ok()
        .filter(|parsed| *parsed != Json::Null)
}

fn add_anthropic_session(data: &mut RequestData, from_headers: &mut Metadata) {
    let Some(body_metadata) = data.get_mut(METADATA_KEY) else {
        return;
    };
    // ast-grep-ignore: no-long-names - B1105 defines this name
    let Some(session_id) = anthropic_session_id_from_metadata(body_metadata) else {
        return;
    };
    if let Json::Object(fields) = body_metadata
        && matches!(fields.get(USER_ID_KEY), Some(Json::Object(_)))
    {
        fields.insert(USER_ID_KEY.to_owned(), Json::Text(session_id.clone()));
    }
    from_headers.insert(SESSION_ID_KEY.to_owned(), Json::Text(session_id.clone()));
    data.insert(LITELLM_SESSION_ID.to_owned(), Json::Text(session_id));
    tracing::debug!("Extracted session_id from Anthropic metadata.user_id");
}

// the W3C headers come last, so a caller's own OpenTelemetry trace still correlates with the proxy's logs
fn add_w3c_fallback(
    headers: &IndexMap<String, String>,
    data: &mut RequestData,
    from_headers: &mut Metadata,
) {
    if !data.contains_key(LITELLM_TRACE_ID)
        && let Some(trace_id) =
            last_header(headers, TRACEPARENT).and_then(trace_id_from_traceparent)
    {
        from_headers.insert(TRACE_ID_KEY.to_owned(), Json::Text(trace_id.clone()));
        tracing::debug!("Extracted trace_id from W3C traceparent header: {trace_id}");
        data.insert(LITELLM_TRACE_ID.to_owned(), Json::Text(trace_id));
    }
    if !data.contains_key(LITELLM_SESSION_ID)
        && let Some(session_id) = last_header(headers, BAGGAGE).and_then(session_id_from_baggage)
    {
        from_headers.insert(SESSION_ID_KEY.to_owned(), Json::Text(session_id.clone()));
        data.insert(LITELLM_SESSION_ID.to_owned(), Json::Text(session_id));
        tracing::debug!("Extracted session_id from W3C baggage header");
    }
}

// a later header of the same name wins, as LiteLLM folds the names to lower case into one dict
fn last_header<'headers>(
    headers: &'headers IndexMap<String, String>,
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .rev()
        .find(|(header, _value)| header.to_lowercase() == name)
        .map(|(_header, value)| value.as_str())
}
