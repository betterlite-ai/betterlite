use std::collections::BTreeSet;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_state::state::{StreamOptions, StreamState};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_chat_output::records::{
    ChoiceRecord, ResponseRecord, ToolCallRecord,
};
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::Payload;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_streaming::streamed_response_assembly::{Request, stream_chunk_builder};

use crate::anthropic_sse::assemble_anthropic_sse_stream::{
    SseAssembler, assemble_anthropic_sse_stream,
};
use crate::anthropic_sse::sse_stream::parsed_sse_events;
use crate::guardrail_message_scope::{Object, StreamItem};
use crate::tool_permission::types::mapping_value;

use super::error::ResponseToolError;
use super::extraction::id_text;
use super::stream_services::{SilentConsole, SilentLogger, StreamServices};

// LiteLLM's stream_chunk_builder over parsed OpenAI chunks; a text or empty response has nothing to check.
pub(crate) async fn assemble_openai_chunks(
    items: &[StreamItem],
    services: &StreamServices<'_>,
) -> Result<Option<Object>, ResponseToolError> {
    let mut chunks = Vec::with_capacity(items.len());
    for item in items {
        let StreamItem::Json(GuardrailValue::Object(fields)) = item else {
            // A chunk that is not a JSON object is outside the declared chunk type, so nothing assembles.
            return Ok(None);
        };
        let encoded = serde_json::to_string(fields)?;
        chunks.push(serde_json::from_str(&encoded)?);
    }
    if chunks.is_empty() {
        return Ok(None);
    }
    match build_response(&chunks, services).await? {
        Some(Response::Chat(chat)) => {
            let value = serde_json::to_value(&chat)?;
            Ok(Some(serde_json::from_value::<Object>(value)?))
        }
        Some(Response::Text(_)) | None => Ok(None),
    }
}

// B2227's builder over the chunk list, behind the silent logger and console the stream check runs with
async fn build_response(
    chunks: &[Chunk],
    services: &StreamServices<'_>,
) -> Result<Option<Response>, ResponseToolError> {
    let request = Request {
        chunks: Some(chunks),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: services.costs,
        counter: services.counter,
        metadata: None,
        now: services.now,
    };
    let hooks = VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::clone(&services.redactor),
    };
    stream_chunk_builder(&request, &hooks)
        .await
        .map_err(ResponseToolError::Assembly)
}

// The Anthropic stream assembles to an OpenAI-shaped record that the check reads and the re-encoder turns back into frames.
pub(crate) struct AssembledAnthropicStream {
    pub(crate) response: Object,
    pub(crate) record: ResponseRecord,
}

// LiteLLM's assemble_anthropic_sse_stream over the shared decoder, identity off, every failure read as no response.
pub(crate) async fn assemble_anthropic_stream(
    items: &[StreamItem],
    services: &StreamServices<'_>,
) -> Option<AssembledAnthropicStream> {
    let assembler = AnthropicAssembler { services };
    let record = assemble_anthropic_sse_stream(items, &assembler, false).await?;
    let response = record_view(&record);
    Some(AssembledAnthropicStream { response, record })
}

// LiteLLM rewrites the assembled response in place, so only the fields a denial changes come back onto the record.
pub(crate) fn rewritten_record(response: &Object, original: &ResponseRecord) -> ResponseRecord {
    let mut record = original.clone();
    let Some(GuardrailValue::Array(choices)) = response.get("choices") else {
        return record;
    };
    for (choice, rewritten) in record.choices.iter_mut().zip(choices) {
        let Some(message) = mapping_value(rewritten, "message") else {
            continue;
        };
        choice.message.content = match mapping_value(message, "content") {
            Some(GuardrailValue::Text(text)) => Some(text.clone()),
            None | Some(_) => None,
        };
        let kept = kept_tool_call_ids(message);
        choice
            .message
            .tool_calls
            .retain(|call| kept.contains(call.id.as_deref().unwrap_or("")));
        choice.finish_reason = match mapping_value(rewritten, "finish_reason") {
            Some(GuardrailValue::Text(reason)) => Some(reason.clone()),
            None | Some(_) => None,
        };
    }
    record
}

// The ids of the calls the rewrite left in the message, which are the only record calls that stay.
fn kept_tool_call_ids(message: &GuardrailValue) -> BTreeSet<&str> {
    match mapping_value(message, "tool_calls") {
        Some(GuardrailValue::Array(calls)) => calls.iter().map(id_text).collect(),
        None | Some(_) => BTreeSet::new(),
    }
}

struct AnthropicAssembler<'services, 'ids> {
    services: &'services StreamServices<'ids>,
}

struct ZeroReasoning;

impl ReasoningTokenCounter for ZeroReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

impl SseAssembler for AnthropicAssembler<'_, '_> {
    fn assemble(
        &self,
        sse_stream: &str,
        _model: &str,
    ) -> impl Future<Output = Option<ResponseRecord>> + Send {
        let events = parsed_sse_events(sse_stream);
        let services = self.services;
        async move { assemble_events(&events, services).await }
    }
}

// The passthrough handler's event decoder over the parsed frames, then B2227's builder over its chunks.
async fn assemble_events(events: &[Json], services: &StreamServices<'_>) -> Option<ResponseRecord> {
    let mut converter = StreamEventConverter {
        state: StreamState::new(StreamOptions::default(), services.ids),
        counter: ZeroReasoning,
        json_could_close: false,
    };
    let mut chunks = Vec::with_capacity(events.len());
    for event in events {
        let chunk = converter.chunk_parser(event).ok()?;
        let mut encoded = serde_json::to_value(&chunk).ok()?;
        // LiteLLM's ModelResponseStream stamps the object and creation time its builder reads from the first chunk
        let fields = encoded.as_object_mut()?;
        fields.insert(
            "object".to_owned(),
            serde_json::json!("chat.completion.chunk"),
        );
        fields.insert("created".to_owned(), serde_json::json!(services.now));
        chunks.push(serde_json::from_value(encoded).ok()?);
    }
    if chunks.is_empty() {
        return None;
    }
    let Some(Response::Chat(chat)) = build_response(&chunks, services).await.ok()? else {
        return None;
    };
    let encoded = serde_json::to_string(&chat).ok()?;
    serde_json::from_str(&encoded).ok()
}

// The rewrite helpers read the OpenAI message shape; every other record field stays with the assembler.
fn record_view(record: &ResponseRecord) -> Object {
    let mut fields = Object::new();
    let choices = record.choices.iter().map(choice_view).collect();
    fields.insert("choices".to_owned(), GuardrailValue::Array(choices));
    fields
}

fn choice_view(choice: &ChoiceRecord) -> GuardrailValue {
    let mut message = Object::new();
    let content = choice
        .message
        .content
        .clone()
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    message.insert("content".to_owned(), content);
    let tool_calls = choice
        .message
        .tool_calls
        .iter()
        .map(tool_call_view)
        .collect();
    message.insert("tool_calls".to_owned(), GuardrailValue::Array(tool_calls));
    let mut fields = Object::new();
    fields.insert("message".to_owned(), GuardrailValue::Object(message));
    let finish_reason = choice
        .finish_reason
        .clone()
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    fields.insert("finish_reason".to_owned(), finish_reason);
    GuardrailValue::Object(fields)
}

fn tool_call_view(call: &ToolCallRecord) -> GuardrailValue {
    let name = call
        .function
        .as_ref()
        .and_then(|function| function.name.clone())
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    let arguments = call
        .function
        .as_ref()
        .and_then(|function| function.arguments.clone())
        .map_or(GuardrailValue::Null, GuardrailValue::Text);
    let mut function = Object::new();
    function.insert("name".to_owned(), name);
    function.insert("arguments".to_owned(), arguments);
    let mut fields = Object::new();
    fields.insert(
        "id".to_owned(),
        call.id
            .clone()
            .map_or(GuardrailValue::Null, GuardrailValue::Text),
    );
    fields.insert(
        "type".to_owned(),
        GuardrailValue::Text("function".to_owned()),
    );
    fields.insert("function".to_owned(), GuardrailValue::Object(function));
    GuardrailValue::Object(fields)
}

mod tests;
