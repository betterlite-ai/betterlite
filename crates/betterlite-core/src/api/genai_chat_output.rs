mod tests;

use std::collections::BTreeMap;
use std::mem::take;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::{AssistantMessage, ChatResponse, ChatUsage};
use betterlite_openai::stream::chunk::{Chunk, Delta, ToolCall};
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

const MODEL_ROLE: &str = "model";
const UNDEFINED_TOOL_NAME: &str = "undefined_tool_name";

// LiteLLM's _map_finish_reason dictionary; an absent or unknown reason maps to STOP
const FINISH_REASONS: [(&str, &str); 5] = [
    ("stop", "STOP"),
    ("length", "MAX_TOKENS"),
    ("content_filter", "SAFETY"),
    ("tool_calls", "STOP"),
    ("function_call", "STOP"),
];

#[derive(Debug, Error)]
pub enum GenAiOutputError {
    #[error("the completion response has no choices")]
    NoChoices,
    #[error("the GenAI response cannot be framed: {source}")]
    Frame {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    pub candidates: Vec<Candidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Content,
    pub finish_reason: Option<&'static str>,
    pub index: u32,
    pub safety_ratings: Vec<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Content {
    pub parts: Vec<Part>,
    pub role: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall,
    },
}

// args keeps the provider's JSON as json.loads returned it
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: u64,
    pub candidates_token_count: u64,
    pub total_token_count: u64,
}

#[derive(Debug, Default)]
pub struct GenAiStreamState {
    accumulated_tool_calls: BTreeMap<u32, ToolCallAccumulator>,
}

#[derive(Debug, Default)]
struct ToolCallAccumulator {
    name: String,
    arguments: String,
}

pub fn translate_completion(
    response: &ChatResponse,
) -> Result<GenerateContentResponse, GenAiOutputError> {
    let choice = response
        .choices
        .first()
        .ok_or(GenAiOutputError::NoChoices)?;
    Ok(GenerateContentResponse {
        candidates: vec![Candidate {
            content: Content {
                parts: message_parts(&choice.message),
                role: MODEL_ROLE,
            },
            finish_reason: Some(map_finish_reason(
                choice.finish_reason.as_deref().unwrap_or_default(),
            )),
            index: 0,
            safety_ratings: Vec::new(),
        }],
        usage_metadata: Some(map_usage(&response.usage)),
    })
}

// LiteLLM skips an empty delta and a chunk whose parts and finish reason are both missing
#[must_use]
pub fn translate_chunk(
    chunk: &Chunk,
    state: &mut GenAiStreamState,
) -> Option<GenerateContentResponse> {
    let choice = chunk.choices.first()?;
    let parts = delta_parts(&choice.delta, state);
    let finish_reason = choice
        .finish_reason
        .as_deref()
        .filter(|reason| !reason.is_empty());
    if parts.is_empty() && finish_reason.is_none() {
        return None;
    }
    let usage_metadata = finish_reason.map(|_reason| {
        chunk
            .usage
            .as_ref()
            .map_or_else(UsageMetadata::default, map_usage)
    });
    Some(GenerateContentResponse {
        candidates: vec![Candidate {
            content: Content {
                parts,
                role: MODEL_ROLE,
            },
            finish_reason: finish_reason.map(map_finish_reason),
            index: 0,
            safety_ratings: Vec::new(),
        }],
        usage_metadata,
    })
}

#[must_use]
pub fn translate_stream(
    stream: BoxStream<'static, Chunk>,
) -> BoxStream<'static, Result<Vec<u8>, GenAiOutputError>> {
    stream::unfold(
        (stream, GenAiStreamState::default()),
        |(mut inner, mut accumulator)| async move {
            while let Some(chunk) = inner.next().await {
                let Some(response) = translate_chunk(&chunk, &mut accumulator) else {
                    continue;
                };
                return Some((sse_frame(&response), (inner, accumulator)));
            }
            None
        },
    )
    .boxed()
}

fn sse_frame(response: &GenerateContentResponse) -> Result<Vec<u8>, GenAiOutputError> {
    let payload =
        serde_json::to_string(response).map_err(|source| GenAiOutputError::Frame { source })?;
    Ok(format!("data: {payload}\n\n").into_bytes())
}

fn message_parts(message: &AssistantMessage) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(content) = message.content.as_ref().filter(|text| !text.is_empty()) {
        parts.push(Part::Text {
            text: content.clone(),
        });
    }
    for tool_call in message.tool_calls.iter().flatten() {
        let arguments = &tool_call.function.arguments;
        let args = if arguments.is_empty() {
            Json::Object(IndexMap::new())
        } else {
            serde_json::from_str::<Json>(arguments)
                .unwrap_or_else(|_error| Json::Object(IndexMap::new()))
        };
        let name = if tool_call.function.name.is_empty() {
            UNDEFINED_TOOL_NAME.to_owned()
        } else {
            tool_call.function.name.clone()
        };
        parts.push(Part::FunctionCall {
            function_call: FunctionCall { name, args },
        });
    }
    if parts.is_empty() {
        parts.push(Part::Text {
            text: String::new(),
        });
    }
    parts
}

fn delta_parts(delta: &Delta, state: &mut GenAiStreamState) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(content) = delta.content.as_ref().filter(|text| !text.is_empty()) {
        parts.push(Part::Text {
            text: content.clone(),
        });
    }
    for tool_call in delta.tool_calls.iter().flatten() {
        accumulate_tool_call(tool_call, state, &mut parts);
    }
    parts
}

// a tool call is emitted once its accumulated arguments parse and a name has arrived
fn accumulate_tool_call(tool_call: &ToolCall, state: &mut GenAiStreamState, parts: &mut Vec<Part>) {
    let Some(function) = tool_call.function.as_ref() else {
        return;
    };
    let delta_name = function.name.as_deref().filter(|value| !value.is_empty());
    if delta_name.is_none() && function.arguments.is_empty() {
        return;
    }
    let accumulated = state
        .accumulated_tool_calls
        .entry(tool_call.index)
        .or_default();
    if let Some(found) = delta_name {
        found.clone_into(&mut accumulated.name);
    }
    accumulated.arguments.push_str(&function.arguments);
    let Ok(args) = serde_json::from_str::<Json>(&accumulated.arguments) else {
        return;
    };
    if accumulated.name.is_empty() {
        return;
    }
    let completed_name = take(&mut accumulated.name);
    parts.push(Part::FunctionCall {
        function_call: FunctionCall {
            name: completed_name,
            args,
        },
    });
    state.accumulated_tool_calls.remove(&tool_call.index);
}

fn map_finish_reason(reason: &str) -> &'static str {
    FINISH_REASONS
        .iter()
        .find(|(raw, _mapped)| *raw == reason)
        .map_or("STOP", |(_raw, mapped)| *mapped)
}

const fn map_usage(usage: &ChatUsage) -> UsageMetadata {
    UsageMetadata {
        prompt_token_count: usage.prompt_tokens,
        candidates_token_count: usage.completion_tokens,
        total_token_count: usage.total_tokens,
    }
}
