use std::io::Read;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod tests;

pub const OTLP_MAX_ATTRIBUTE_VALUE_BYTES: usize = 0x1_0000;
pub const OTLP_MAX_BODY_BYTES: usize = 0x0040_0000;

#[derive(Debug, Error)]
pub enum OtlpError {
    #[error("OTLP body exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("Unsupported OTLP content encoding")]
    UnsupportedEncoding,
    #[error("Invalid OTLP gzip body")]
    InvalidGzip(#[source] std::io::Error),
    #[error("Invalid OTLP payload: {0}")]
    InvalidPayload(String),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NormalizedSpan {
    #[serde(default)]
    pub start_ns: i64,
    #[serde(default)]
    pub end_ns: i64,
    #[serde(default)]
    pub trace_id: String,
    #[serde(default)]
    pub span_id: String,
    #[serde(default)]
    pub parent_span_id: String,
    #[serde(default)]
    pub trace_state: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub resource_attributes: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub scope_name: String,
    #[serde(default)]
    pub scope_version: String,
    #[serde(default)]
    pub attributes: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub consumed_attributes: Vec<String>,
    #[serde(default)]
    pub status_code: String,
    #[serde(default)]
    pub status_message: String,
    #[serde(default)]
    pub observation_type: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub litellm_request_id: String,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub events: Vec<SpanEvent>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SpanEvent {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub attributes: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanRow {
    pub timestamp: i64,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub trace_state: String,
    pub span_name: String,
    pub span_kind: String,
    pub service_name: String,
    pub resource_attributes: std::collections::BTreeMap<String, String>,
    pub scope_name: String,
    pub scope_version: String,
    pub span_attributes: std::collections::BTreeMap<String, String>,
    pub duration: i64,
    pub status_code: String,
    pub status_message: String,
    pub team_id: String,
    pub api_key_hash: String,
    pub observation_type: String,
    pub agent_name: String,
    pub model: String,
    pub litellm_request_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub input: String,
    pub output: String,
}

pub fn decode_content_encoding(body: &[u8], encoding: Option<&str>) -> Result<Vec<u8>, OtlpError> {
    if body.len() > OTLP_MAX_BODY_BYTES {
        return Err(OtlpError::TooLarge {
            limit: OTLP_MAX_BODY_BYTES,
        });
    }
    match encoding.map(str::to_ascii_lowercase).as_deref() {
        None | Some("identity") => Ok(body.to_vec()),
        Some("gzip") => decode_gzip(body),
        Some(_) => Err(OtlpError::UnsupportedEncoding),
    }
}

pub fn decode_otlp(
    body: &[u8],
    _content_type: Option<&str>,
    encoding: Option<&str>,
) -> Result<Vec<SpanRow>, OtlpError> {
    let payload = decode_content_encoding(body, encoding)?;
    let spans: Vec<NormalizedSpan> = serde_json::from_slice(&payload)
        .map_err(|error| OtlpError::InvalidPayload(error.to_string()))?;
    Ok(spans.iter().map(span_row).collect())
}

fn decode_gzip(body: &[u8]) -> Result<Vec<u8>, OtlpError> {
    let mut gzip_reader = GzDecoder::new(body);
    let limit = u64::try_from(OTLP_MAX_BODY_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut limited = gzip_reader.by_ref().take(limit);
    let mut payload = Vec::new();
    limited
        .read_to_end(&mut payload)
        .map_err(OtlpError::InvalidGzip)?;
    if payload.len() > OTLP_MAX_BODY_BYTES {
        return Err(OtlpError::TooLarge {
            limit: OTLP_MAX_BODY_BYTES,
        });
    }
    Ok(payload)
}

pub fn truncate(value: &str) -> String {
    let encoded = value.as_bytes();
    if encoded.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES {
        return value.to_owned();
    }
    let kept = String::from_utf8_lossy(&encoded[..OTLP_MAX_ATTRIBUTE_VALUE_BYTES]);
    let kept_bytes = kept.len();
    format!(
        "{kept}…[truncated {} bytes]",
        encoded.len().saturating_sub(kept_bytes)
    )
}

pub fn span_row(span: &NormalizedSpan) -> SpanRow {
    let attributes = span
        .attributes
        .iter()
        .filter(|(key, _)| !span.consumed_attributes.contains(key))
        .map(|(key, value)| (key.clone(), truncate(value)))
        .collect();
    let status_message = if span.status_message.is_empty() {
        exception_message(span)
    } else {
        span.status_message.clone()
    };
    SpanRow {
        timestamp: span.start_ns,
        trace_id: span.trace_id.clone(),
        span_id: span.span_id.clone(),
        parent_span_id: span.parent_span_id.clone(),
        trace_state: span.trace_state.clone(),
        span_name: span.name.clone(),
        span_kind: span.kind.clone(),
        service_name: span
            .resource_attributes
            .get("service.name")
            .cloned()
            .unwrap_or_default(),
        resource_attributes: span.resource_attributes.clone(),
        scope_name: span.scope_name.clone(),
        scope_version: span.scope_version.clone(),
        span_attributes: attributes,
        duration: span.end_ns.saturating_sub(span.start_ns),
        status_code: span.status_code.clone(),
        status_message,
        team_id: String::new(),
        api_key_hash: String::new(),
        observation_type: span.observation_type.clone(),
        agent_name: span.agent_name.clone(),
        model: span.model.clone(),
        litellm_request_id: span
            .attributes
            .get("gen_ai.response.id")
            .cloned()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| span.litellm_request_id.clone()),
        input_tokens: span.input_tokens,
        output_tokens: span.output_tokens,
        input: truncate_payload(&span.input),
        output: truncate(&span.output),
    }
}

fn exception_message(span: &NormalizedSpan) -> String {
    let message = span
        .events
        .iter()
        .find(|event| event.name == "exception")
        .and_then(|event| {
            event
                .attributes
                .get("exception.message")
                .or_else(|| event.attributes.get("exception.type"))
        })
        .cloned()
        .unwrap_or_default();
    message
}

pub fn truncate_payload(value: &str) -> String {
    if value.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES || !value.starts_with('[') {
        return truncate(value);
    }
    let Ok(messages) = serde_json::from_str::<Vec<serde_json::Value>>(value) else {
        return truncate(value);
    };
    if messages.len() < 2 {
        return truncate(value);
    }
    let encoded: Vec<String> = messages
        .iter()
        .map(|message| serde_json::to_string(message).unwrap_or_default())
        .collect();
    let marker = |count: usize| serde_json::json!({"role":"system","content":format!("…[{count} earlier messages truncated]")});
    let first = &encoded[0];
    let mut tail = Vec::new();
    let mut used = first.len()
        + 1
        + serde_json::to_string(&marker(messages.len() - 1))
            .unwrap_or_default()
            .len()
        + 1;
    for message in encoded.iter().skip(1).rev() {
        if used.saturating_add(message.len()).saturating_add(2) > OTLP_MAX_ATTRIBUTE_VALUE_BYTES {
            break;
        }
        used = used.saturating_add(message.len()).saturating_add(1);
        tail.push(message.clone());
    }
    tail.reverse();
    let omitted = messages.len().saturating_sub(tail.len()).saturating_sub(1);
    let mut output = vec![
        first.clone(),
        serde_json::to_string(&marker(omitted)).unwrap_or_default(),
    ];
    output.extend(tail);
    let candidate = format!("[{}]", output.join(", "));
    if candidate.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES {
        return candidate;
    }
    format!(
        "[{}]",
        serde_json::to_string(&marker(messages.len())).unwrap_or_default()
    )
}
