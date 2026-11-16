use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_core::guardrails::model_armor::streams::assembly::Assembly;
use betterlite_core::guardrails::model_armor::streams::chunk::{Chunk, field, text_field};
use betterlite_core::guardrails::model_armor::streams::errors::HttpError;
use betterlite_server::guardrails::anthropic_sse::sse_chunks_from_response::sse_chunks_from_response;

use super::WireError;

const DATA_PREFIX: &str = "data:";
const COMPLETION_OBJECT: &str = "chat.completion";

#[derive(Default)]
struct Assembled {
    id: Option<String>,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    text: String,
    stop_reason: Option<String>,
    started: bool,
}

// LiteLLM's assemble_anthropic_sse_stream, reduced to the fields a scan and a replay read.
pub(super) fn assemble(chunks: &[Chunk]) -> Option<Assembly> {
    let mut state = Assembled::default();
    for chunk in chunks {
        let Chunk::Bytes(bytes) = chunk else {
            continue;
        };
        let Ok(frame) = from_utf8(bytes) else {
            continue;
        };
        let Some(event) = event_payload(frame) else {
            continue;
        };
        absorb(&event, &mut state);
    }
    if !state.started {
        return None;
    }
    Some(Assembly::Model(state.into_body()))
}

fn event_payload(frame: &str) -> Option<GuardrailValue> {
    let data = frame
        .lines()
        .find_map(|line| line.strip_prefix(DATA_PREFIX))?;
    serde_json::from_str(data.trim()).ok()
}

fn absorb(event: &GuardrailValue, state: &mut Assembled) {
    match text_field(field(event, "type")) {
        Some("message_start") => absorb_start(event, state),
        Some("content_block_delta") => absorb_delta(event, state),
        Some("message_delta") => absorb_stop(event, state),
        Some(_) | None => {}
    }
}

fn absorb_start(event: &GuardrailValue, state: &mut Assembled) {
    state.started = true;
    let Some(message) = field(event, "message") else {
        return;
    };
    if let Some(GuardrailValue::Text(id)) = field(message, "id") {
        state.id = Some(id.clone());
    }
    if let Some(GuardrailValue::Text(model)) = field(message, "model") {
        state.model = Some(model.clone());
    }
    if let Some(accounting) = field(message, "usage") {
        state.input_tokens = integer(field(accounting, "input_tokens"));
        state.output_tokens = integer(field(accounting, "output_tokens"));
    }
}

fn absorb_delta(event: &GuardrailValue, state: &mut Assembled) {
    let Some(delta) = field(event, "delta") else {
        return;
    };
    if let Some(GuardrailValue::Text(text)) = field(delta, "text") {
        state.text.push_str(text);
    }
}

fn absorb_stop(event: &GuardrailValue, state: &mut Assembled) {
    if let Some(delta) = field(event, "delta")
        && let Some(GuardrailValue::Text(reason)) = field(delta, "stop_reason")
    {
        state.stop_reason = Some(reason.clone());
    }
    if let Some(accounting) = field(event, "usage")
        && let Some(tokens) = integer(field(accounting, "output_tokens"))
    {
        state.output_tokens = Some(tokens);
    }
}

fn integer(value: Option<&GuardrailValue>) -> Option<i64> {
    match value {
        Some(GuardrailValue::SignedInteger(number)) => Some(*number),
        Some(GuardrailValue::UnsignedInteger(number)) => i64::try_from(*number).ok(),
        Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        )
        | None => None,
    }
}

fn openai_reason(reason: &str) -> &str {
    match reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

impl Assembled {
    fn into_body(self) -> GuardrailValue {
        let finish_reason = match &self.stop_reason {
            Some(reason) => openai_reason(reason).to_owned(),
            None => "stop".to_owned(),
        };
        let mut message = BTreeMap::new();
        message.insert(
            "role".to_owned(),
            GuardrailValue::Text("assistant".to_owned()),
        );
        message.insert("content".to_owned(), GuardrailValue::Text(self.text));
        let mut choice = BTreeMap::new();
        choice.insert("index".to_owned(), GuardrailValue::SignedInteger(0));
        choice.insert("message".to_owned(), GuardrailValue::Object(message));
        choice.insert(
            "finish_reason".to_owned(),
            GuardrailValue::Text(finish_reason),
        );
        let mut body = BTreeMap::new();
        if let Some(id) = self.id {
            body.insert("id".to_owned(), GuardrailValue::Text(id));
        }
        if let Some(model) = self.model {
            body.insert("model".to_owned(), GuardrailValue::Text(model));
        }
        body.insert(
            "object".to_owned(),
            GuardrailValue::Text(COMPLETION_OBJECT.to_owned()),
        );
        body.insert(
            "choices".to_owned(),
            GuardrailValue::Array(vec![GuardrailValue::Object(choice)]),
        );
        body.insert(
            "usage".to_owned(),
            GuardrailValue::Object(usage(self.input_tokens, self.output_tokens)),
        );
        GuardrailValue::Object(body)
    }
}

fn usage(input: Option<i64>, output: Option<i64>) -> BTreeMap<String, GuardrailValue> {
    let mut usage = BTreeMap::new();
    usage.insert(
        "prompt_tokens".to_owned(),
        GuardrailValue::SignedInteger(input.unwrap_or(0)),
    );
    usage.insert(
        "completion_tokens".to_owned(),
        GuardrailValue::SignedInteger(output.unwrap_or(0)),
    );
    usage
}

// LiteLLM's anthropic_sse_chunks_from_response: the sanitized response replayed as Anthropic frames.
pub(super) fn emit(body: &GuardrailValue) -> Result<Vec<Chunk>, WireError> {
    let plain = serde_json::to_value(body).map_err(WireError::Decode)?;
    let record: ResponseRecord = serde_json::from_value(plain).map_err(WireError::Decode)?;
    let frames = sse_chunks_from_response(&record)?;
    Ok(frames.into_iter().map(Chunk::Bytes).collect())
}

// LiteLLM's build_stream_error_items: one ErrorEvent carrying the status the frame would otherwise lose.
pub(super) fn error_item(error: &HttpError) -> GuardrailValue {
    let mut detail = BTreeMap::new();
    detail.insert(
        "type".to_owned(),
        GuardrailValue::Text("guardrail_error".to_owned()),
    );
    detail.insert(
        "code".to_owned(),
        GuardrailValue::Text(error.status.to_string()),
    );
    detail.insert(
        "message".to_owned(),
        GuardrailValue::Text(detail_message(&error.detail)),
    );
    detail.insert("param".to_owned(), GuardrailValue::Null);
    let mut item = BTreeMap::new();
    item.insert("type".to_owned(), GuardrailValue::Text("error".to_owned()));
    item.insert(
        "sequence_number".to_owned(),
        GuardrailValue::UnsignedInteger(0),
    );
    item.insert("error".to_owned(), GuardrailValue::Object(detail));
    GuardrailValue::Object(item)
}

fn detail_message(detail: &GuardrailValue) -> String {
    let GuardrailValue::Object(fields) = detail else {
        return plain_text(detail);
    };
    if let Some(GuardrailValue::Text(error)) = fields.get("error") {
        return error.clone();
    }
    if let Some(GuardrailValue::Object(error)) = fields.get("error")
        && let Some(GuardrailValue::Text(message)) = error.get("message")
    {
        return message.clone();
    }
    if let Some(GuardrailValue::Text(message)) = fields.get("message") {
        return message.clone();
    }
    serde_json::to_string(detail).unwrap_or_default()
}

fn plain_text(detail: &GuardrailValue) -> String {
    match detail {
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => serde_json::to_string(detail).unwrap_or_default(),
    }
}
