use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta, Function, Logprobs, ToolCall};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
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
    function: Option<Function>,
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
    if serde_json::from_str::<serde::de::IgnoredAny>(text).is_err() {
        return Ok(Projection::Pending);
    }
    if is_messages_api {
        return match serde_json::from_str::<ChatEvent>(text) {
            Ok(event) => Ok(Projection::Ready(SagemakerChunk::Chat(
                event.into_chunk(model),
            ))),
            Err(source) => Err(SagemakerStreamError::Json(source)),
        };
    }
    match serde_json::from_str::<TokenEvent>(text) {
        Ok(event) => Ok(Projection::Ready(SagemakerChunk::Token(event.into_chunk()))),
        Err(source) => Err(SagemakerStreamError::Json(source)),
    }
}

pub enum Projection {
    Ready(SagemakerChunk),
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
        Choice {
            index: self.index,
            delta: self.delta.unwrap_or_default().into_delta(),
            // StreamingChoices maps a truthy stop reason and stores None for an empty one
            finish_reason: self
                .finish_reason
                .filter(|raw| !raw.is_empty())
                .map(|raw| map_finish_reason(&raw).reason.as_str().to_owned()),
            logprobs: self.logprobs,
        }
    }
}

impl DeltaEvent {
    fn into_delta(self) -> Delta {
        // Delta.__init__ numbers a tool call without an index in list order and types it function
        let mut tool_calls = None;
        if let Some(calls) = self.tool_calls {
            let mut numbered = Vec::with_capacity(calls.len());
            let mut next_index = 0;
            for call in calls {
                let index = match call.index {
                    Some(index) => index,
                    None => {
                        let index = next_index;
                        next_index += 1;
                        index
                    }
                };
                numbered.push(ToolCall {
                    index,
                    id: call.id,
                    kind: Some(call.kind.unwrap_or_else(|| String::from("function"))),
                    function: call.function,
                });
            }
            tool_calls = Some(numbered);
        }
        Delta {
            role: self.role,
            content: self.content,
            refusal: self.refusal,
            tool_calls,
            function_call: self.function_call,
        }
    }
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
