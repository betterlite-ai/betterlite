use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::chunk::{Chunk, field, text_field};
use super::surface::Surface;
use super::wire::StreamWire;

// A refusal the buffered hook frames as the stream's terminal item.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpError {
    pub status: u16,
    pub detail: GuardrailValue,
}

// What a buffered stream run can fail with: a frame that cannot be encoded, or a failure the wiring raises.
#[derive(Debug, thiserror::Error)]
pub enum StreamError<Failure: 'static> {
    #[error("the guardrail error frame cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Wiring(Failure),
}

// LiteLLM's _error_payload: the error object a terminal stream item carries, with the status a frame would otherwise lose.
pub fn error_payload(error: &HttpError) -> Result<GuardrailValue, serde_json::Error> {
    let mut payload = match &error.detail {
        GuardrailValue::Object(fields) => fields.clone(),
        other => message_fields(other)?,
    };
    let error_value = match payload.remove("error") {
        Some(value) => value,
        None => GuardrailValue::Object(payload),
    };
    let mut fields = match error_value {
        GuardrailValue::Object(fields) => fields,
        other => message_fields(&other)?,
    };
    fields.insert(
        "code".to_owned(),
        GuardrailValue::Text(error.status.to_string()),
    );
    Ok(GuardrailValue::Object(fields))
}

fn message_fields(
    detail: &GuardrailValue,
) -> Result<BTreeMap<String, GuardrailValue>, serde_json::Error> {
    let text = match detail {
        GuardrailValue::Text(text) => text.clone(),
        other => serde_json::to_string(other)?,
    };
    let mut fields = BTreeMap::new();
    fields.insert("message".to_owned(), GuardrailValue::Text(text));
    Ok(fields)
}

// LiteLLM's _stream_error_items: the endpoint's own error wire format, and plain SSE frames where it has none.
pub fn frame_error<W: StreamWire>(
    wire: &W,
    surface: Surface,
    error: &HttpError,
) -> Result<Vec<Chunk>, StreamError<W::Failure>> {
    let payload = error_payload(error).map_err(StreamError::Encode)?;
    match surface {
        Surface::AnthropicMessages => {
            let message = text_field(field(&payload, "message")).unwrap_or_default();
            wire.anthropic_error_frames(message)
                .map_err(StreamError::Wiring)
        }
        Surface::Responses => match wire.responses_error_items(error) {
            Some(items) if !items.is_empty() => Ok(items),
            _ => sse_frame(&payload).map_err(StreamError::Encode),
        },
        Surface::ChatCompletions | Surface::OpaqueSse => {
            sse_frame(&payload).map_err(StreamError::Encode)
        }
    }
}

// The fallback frame: create_response() reads the status back out of this form, so the refusal keeps its code.
pub fn sse_frame(payload: &GuardrailValue) -> Result<Vec<Chunk>, serde_json::Error> {
    let mut wrapper = BTreeMap::new();
    wrapper.insert("error".to_owned(), payload.clone());
    let body = serde_json::to_string(&GuardrailValue::Object(wrapper))?;
    Ok(vec![Chunk::Bytes(format!("data: {body}\n\n").into_bytes())])
}
