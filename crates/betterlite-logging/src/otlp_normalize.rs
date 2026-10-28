use std::borrow::Cow;
use std::io::Read as _;

use flate2::bufread::GzDecoder;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::otlp_errors::{CodecError, OtlpError};
use crate::otlp_payload::truncate_payload;
use crate::trace_types::SpanType;

mod tests;

// The litellm.constants defaults, 64 KiB per stored attribute and 16 MiB per request body
pub const OTLP_MAX_ATTRIBUTE_VALUE_BYTES: usize = 0x1_0000;
pub const OTLP_MAX_BODY_BYTES: usize = 0x0100_0000;

// LiteLLM's native bridge parses OTLP protobuf and JSON into spans; ingest only normalizes what it returns
pub trait SpanCodec {
    fn decode(
        &self,
        payload: &[u8],
        content_type: Option<&str>,
    ) -> Result<Vec<DecodedSpan>, CodecError>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecodedEvent {
    pub name: String,
    pub attributes: IndexMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NormalizedSpan {
    pub observation_type: SpanType,
    pub agent_name: String,
    pub litellm_request_id: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub input: String,
    pub output: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecodedSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub trace_state: String,
    pub name: String,
    pub kind: String,
    pub resource_attributes: IndexMap<String, String>,
    pub scope_name: String,
    pub scope_version: String,
    pub attributes: IndexMap<String, String>,
    pub start_ns: i64,
    pub end_ns: i64,
    pub status_code: String,
    pub status_message: String,
    pub events: Vec<DecodedEvent>,
    pub normalized: NormalizedSpan,
    pub consumed_attributes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SpanRow {
    pub timestamp: i64,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub trace_state: String,
    pub span_name: String,
    pub span_kind: String,
    pub service_name: String,
    pub resource_attributes: IndexMap<String, String>,
    pub scope_name: String,
    pub scope_version: String,
    pub span_attributes: IndexMap<String, String>,
    pub duration: i64,
    pub status_code: String,
    pub status_message: String,
    pub team_id: String,
    pub api_key_hash: String,
    pub observation_type: SpanType,
    pub agent_name: String,
    pub model: String,
    #[serde(rename = "LiteLLMRequestId")]
    pub litellm_request_id: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub input: String,
    pub output: String,
}

pub fn decode_otlp<Codec>(
    codec: &Codec,
    body: &[u8],
    content_type: Option<&str>,
    content_encoding: Option<&str>,
) -> Result<Vec<SpanRow>, OtlpError>
where
    Codec: SpanCodec,
{
    let payload = decode_content_encoding(body, content_encoding)?;
    let spans = codec
        .decode(&payload, content_type)
        .map_err(|error| match error {
            CodecError::Overflow(message) => OtlpError::SpansTooLarge(message),
            CodecError::Malformed(message) => OtlpError::InvalidSpans(message),
        })?;
    Ok(spans.iter().map(span_row).collect())
}

pub fn decode_content_encoding<'body>(
    body: &'body [u8],
    content_encoding: Option<&str>,
) -> Result<Cow<'body, [u8]>, OtlpError> {
    if body.len() > OTLP_MAX_BODY_BYTES {
        return Err(OtlpError::BodyTooLarge);
    }
    match content_encoding.map(str::to_lowercase).as_deref() {
        None | Some("identity") => Ok(Cow::Borrowed(body)),
        Some("gzip") => decode_gzip(body).map(Cow::Owned),
        Some(_) => Err(OtlpError::UnsupportedEncoding),
    }
}

// Python's gzip module reads members until the body ends and skips zero padding after each one
fn decode_gzip(body: &[u8]) -> Result<Vec<u8>, OtlpError> {
    // One byte past the limit is enough to tell an oversized body without inflating all of it
    let limit = OTLP_MAX_BODY_BYTES.saturating_add(1);
    let mut payload = Vec::new();
    let mut rest = body;
    while !rest.is_empty() && payload.len() < limit {
        let budget = u64::try_from(limit.saturating_sub(payload.len())).unwrap_or(u64::MAX);
        let mut member = GzDecoder::new(rest);
        (&mut member)
            .take(budget)
            .read_to_end(&mut payload)
            .map_err(OtlpError::InvalidGzip)?;
        rest = member.into_inner();
        let padding = rest.iter().take_while(|&&byte| byte == 0).count();
        rest = rest.get(padding..).unwrap_or_default();
    }
    if payload.len() > OTLP_MAX_BODY_BYTES {
        return Err(OtlpError::BodyTooLarge);
    }
    Ok(payload)
}

#[must_use]
pub fn truncate(value: &str) -> String {
    if value.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES {
        return value.to_owned();
    }
    let boundary = value.floor_char_boundary(OTLP_MAX_ATTRIBUTE_VALUE_BYTES);
    let kept = value.get(..boundary).unwrap_or_default();
    format!(
        "{kept}\u{2026}[truncated {} bytes]",
        value.len().saturating_sub(kept.len())
    )
}

#[must_use]
pub fn span_row(span: &DecodedSpan) -> SpanRow {
    let normalized = &span.normalized;
    let request_id = span
        .attributes
        .get("gen_ai.response.id")
        .filter(|id| !id.is_empty())
        .unwrap_or(&normalized.litellm_request_id);
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
        span_attributes: stored_attributes(span),
        duration: span.end_ns.saturating_sub(span.start_ns),
        status_code: span.status_code.clone(),
        status_message: status_message(span),
        team_id: String::new(),
        api_key_hash: String::new(),
        observation_type: normalized.observation_type,
        agent_name: normalized.agent_name.clone(),
        model: normalized.model.clone(),
        litellm_request_id: request_id.clone(),
        input_tokens: normalized.input_tokens,
        output_tokens: normalized.output_tokens,
        input: truncate_payload(&normalized.input),
        output: truncate(&normalized.output),
    }
}

// The codec already moved the consumed attributes into normalized fields, so only the rest is stored
fn stored_attributes(span: &DecodedSpan) -> IndexMap<String, String> {
    let mut attributes = IndexMap::new();
    for (key, value) in &span.attributes {
        if !span.consumed_attributes.contains(key) {
            attributes.insert(key.clone(), truncate(value));
        }
    }
    attributes
}

fn status_message(span: &DecodedSpan) -> String {
    if !span.status_message.is_empty() {
        return span.status_message.clone();
    }
    let Some(event) = span.events.iter().find(|event| event.name == "exception") else {
        return String::new();
    };
    event
        .attributes
        .get("exception.message")
        .filter(|message| !message.is_empty())
        .or_else(|| event.attributes.get("exception.type"))
        .cloned()
        .unwrap_or_default()
}
