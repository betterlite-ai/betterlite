use betterlite_logging::python_json::dumps_object;
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Json as ArgumentsJson;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta, Function, Logprobs, ToolCall};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::sagemaker_stream::SagemakerChunk;
use crate::sagemaker_stream::error::SagemakerStreamError;

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const END_OF_TEXT: &str = "<|endoftext|>";
pub const STOP_REASON: &str = "stop";

// _chunk_parser_messages_api: the event carries OpenAI chunk fields and StreamingChoices fills what it omits
#[derive(Debug, Deserialize)]
pub struct ChatEvent {
    id: String,
    created: u64,
    model: Option<String>,
    choices: Vec<ChoiceEvent>,
    usage: Option<ChatUsage>,
    system_fingerprint: Option<String>,
    service_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChoiceEvent {
    #[serde(default)]
    index: u32,
    delta: Option<DeltaEvent>,
    finish_reason: Option<String>,
    logprobs: Option<Logprobs>,
}

#[derive(Debug, Default, Deserialize)]
struct DeltaEvent {
    role: Option<String>,
    content: Option<String>,
    refusal: Option<String>,
    tool_calls: Option<Vec<ToolCallEvent>>,
    function_call: Option<Function>,
}

#[derive(Debug, Deserialize)]
struct ToolCallEvent {
    index: Option<u32>,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<FunctionEvent>,
}

// Function declares dict | str | None for arguments and normalizes each: a dict dumps, a null is empty
#[derive(Debug, Deserialize)]
struct FunctionEvent {
    name: Option<String>,
    arguments: Option<ArgumentsEvent>,
    parameters: Option<ArgumentsJson>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentsEvent {
    Text(String),
    Object(IndexMap<String, ArgumentsJson>),
}

// _chunk_parser: a missing null token is empty, index counts from zero, and the end token finishes the stream
#[derive(Debug, Default, Deserialize)]
pub struct TokenEvent {
    token: Option<TokenText>,
    index: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct TokenText {
    text: Option<String>,
}

// json.loads gates the buffer: a partial text stays for the next event, a shape error is a projection failure
pub fn project(
    model: &str,
    is_messages_api: bool,
    text: &str,
) -> Result<Projection, SagemakerStreamError> {
    // a struct reads an array positionally, so the object map gates both shapes; partial text waits
    let complete = serde_json::from_str::<ArgumentsJson>(text).is_ok();
    match serde_json::from_str::<IndexMap<String, ArgumentsJson>>(text) {
        Ok(_chunk) => {}
        Err(_incomplete) if !complete => return Ok(Projection::Pending),
        Err(source) => return Err(SagemakerStreamError::Json(source)),
    }
    if is_messages_api {
        return match serde_json::from_str::<ChatEvent>(text) {
            Ok(event) => Ok(Projection::Ready(Box::new(SagemakerChunk::Chat(
                event.into_chunk(model),
            )))),
            Err(source) => Err(SagemakerStreamError::Json(source)),
        };
    }
    match serde_json::from_str::<TokenEvent>(text) {
        Ok(event) => Ok(Projection::Ready(Box::new(SagemakerChunk::Token(
            event.into_chunk(),
        )))),
        Err(source) => Err(SagemakerStreamError::Json(source)),
    }
}

pub enum Projection {
    // boxed: a chat chunk dwarfs the pending marker
    Ready(Box<SagemakerChunk>),
    Pending,
}

impl ChatEvent {
    fn into_chunk(self, model: &str) -> Chunk {
        Chunk {
            id: self.id,
            object: String::from(CHUNK_OBJECT),
            created: self.created,
            model: self.model.unwrap_or_else(|| model.to_owned()),
            choices: self
                .choices
                .into_iter()
                .map(ChoiceEvent::into_choice)
                .collect(),
            usage: self.usage,
            system_fingerprint: self.system_fingerprint,
            service_tier: self.service_tier,
        }
    }
}

impl ChoiceEvent {
    fn into_choice(self) -> Choice {
        let delta = self.delta.unwrap_or_default();
        Choice {
            index: self.index,
            delta: Delta {
                role: delta.role,
                content: delta.content,
                refusal: delta.refusal,
                tool_calls: delta.tool_calls.map(number_calls),
                function_call: delta.function_call,
                reasoning_content: None,
                thinking_blocks: None,
            },
            // StreamingChoices maps a truthy stop reason and stores None for an empty one
            finish_reason: self
                .finish_reason
                .filter(|raw| !raw.is_empty())
                .map(|raw| map_finish_reason(&raw).reason.as_str().to_owned()),
            logprobs: self.logprobs,
        }
    }
}

// Delta.__init__ numbers a tool call without an index in list order and types it function
fn number_calls(calls: Vec<ToolCallEvent>) -> Vec<ToolCall> {
    let mut numbered = Vec::with_capacity(calls.len());
    let mut next_index = 0_u32;
    for call in calls {
        let index = call.index.unwrap_or(next_index);
        if call.index.is_none() {
            next_index = next_index.saturating_add(1);
        }
        numbered.push(ToolCall {
            index,
            id: call.id,
            kind: Some(call.kind.unwrap_or_else(|| String::from("function"))),
            function: call.function.map(|function| Function {
                name: function.name,
                arguments: match function.arguments {
                    Some(ArgumentsEvent::Text(text)) => text,
                    Some(ArgumentsEvent::Object(object)) => dumps_object(&object),
                    // a missing arguments falls back to a dict parameters, and any other value is empty
                    None => match function.parameters {
                        Some(ArgumentsJson::Object(object)) => dumps_object(&object),
                        _ => String::new(),
                    },
                },
            }),
        });
    }
    numbered
}

impl TokenEvent {
    fn into_chunk(self) -> GenericChunk {
        let text = self.token.and_then(|token| token.text).unwrap_or_default();
        let finished = text == END_OF_TEXT;
        GenericChunk {
            text: if finished { String::new() } else { text },
            tool_use: None,
            is_finished: finished,
            finish_reason: if finished {
                String::from(STOP_REASON)
            } else {
                String::new()
            },
            usage: None,
            index: Some(self.index.unwrap_or(0)),
            provider_specific_fields: None,
        }
    }
}
mod tests;
