use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

use super::error::OllamaError;
use super::ids::IdSource;
use super::usage::Usage;

const CHUNK_OBJECT: &str = "chat.completion.chunk";
const DATA_PREFIX: &str = "data: ";
const DATA_PREFIX_WITHOUT_SPACE: &str = "data:";
const DONE_MARKER: &str = "[DONE]";
const TOOL_CALL_TYPE: &str = "function";
const STOP_FINISH_REASON: &str = "stop";
const TOOL_CALLS_FINISH_REASON: &str = "tool_calls";
const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";
const REQUIRED_CHUNK_FIELDS: [&str; 3] = ["message", "done", "model"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReasoningState {
    #[default]
    Idle,
    Started,
    Finished,
}

#[derive(Debug, Default)]
pub struct OllamaChatIterator {
    reasoning: ReasoningState,
    seen_tool_calls: bool,
}

#[derive(Debug)]
pub enum OllamaStreamStep {
    Chunk(Box<OllamaStreamChunk>),
    Skipped,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaStreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: StreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamDelta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaToolCall {
    pub id: Option<String>,
    pub kind: String,
    pub index: u32,
    pub function: DeltaFunction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaFunction {
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
struct RawChunk {
    model: String,
    message: RawMessage,
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<RawToolCall>>,
}

#[derive(Debug, Deserialize)]
struct RawToolCall {
    function: RawFunction,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    index: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Json>,
    #[serde(default)]
    parameters: Option<Json>,
}

impl OllamaChatIterator {
    // B2214's _handle_string_chunk over one transport line, then chunk_parser
    pub fn next_chunk(
        &mut self,
        line: &str,
        now: i64,
        ids: &dyn IdSource,
    ) -> Result<OllamaStreamStep, OllamaError> {
        if line.trim().is_empty() {
            return Ok(OllamaStreamStep::Skipped);
        }
        if line.contains(DONE_MARKER) {
            return Ok(OllamaStreamStep::Done);
        }
        let payload = line
            .strip_prefix(DATA_PREFIX)
            .or_else(|| line.strip_prefix(DATA_PREFIX_WITHOUT_SPACE))
            .unwrap_or(line);
        let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
            return Ok(OllamaStreamStep::Skipped);
        };
        if !parsed.is_truthy() {
            return Ok(OllamaStreamStep::Skipped);
        }
        let chunk = self.chunk_parser(payload.as_bytes(), now, ids)?;
        Ok(OllamaStreamStep::Chunk(Box::new(chunk)))
    }

    // LiteLLM's chunk_parser: a message, done and model key raise the 400 OllamaError, anything else the generic 500
    pub fn chunk_parser(
        &mut self,
        chunk: &[u8],
        now: i64,
        ids: &dyn IdSource,
    ) -> Result<OllamaStreamChunk, OllamaError> {
        let parsed = serde_json::from_slice::<Json>(chunk)
            .map_err(|source| OllamaError::Shape { source })?;
        let Json::Object(object) = parsed else {
            return Err(OllamaError::ChunkField { field: "chunk" });
        };
        for field in REQUIRED_CHUNK_FIELDS {
            if !object.contains_key(field) {
                return Err(OllamaError::ChunkMissing { field });
            }
        }
        let wire =
            serde_json::to_string(&object).map_err(|source| OllamaError::Shape { source })?;
        let raw: RawChunk =
            serde_json::from_str(&wire).map_err(|source| OllamaError::Shape { source })?;
        let delta = self.stream_delta(&raw.message, ids)?;
        let finish_reason = raw.done.then(|| self.finish_reason(&raw));
        let usage = done_chunk_usage(&raw);
        Ok(OllamaStreamChunk {
            id: ids.next_id(),
            object: String::from(CHUNK_OBJECT),
            created: now,
            model: raw.model,
            choices: vec![StreamChoice {
                index: 0,
                delta,
                finish_reason,
            }],
            usage,
        })
    }

    fn stream_delta(
        &mut self,
        message: &RawMessage,
        ids: &dyn IdSource,
    ) -> Result<StreamDelta, OllamaError> {
        let mut delta = StreamDelta::default();
        if let Some(calls) = message.tool_calls.as_ref() {
            self.seen_tool_calls = true;
            delta.tool_calls = Some(delta_tool_calls(calls, ids)?);
        }
        if let Some(thinking) = message.thinking.as_deref().filter(|text| !text.is_empty()) {
            delta.reasoning_content = Some(thinking.to_owned());
            self.start_reasoning();
        }
        if let Some(content) = message.content.clone().filter(|text| !text.is_empty()) {
            self.content_delta(&mut delta, content);
        }
        Ok(delta)
    }

    fn start_reasoning(&mut self) {
        if self.reasoning == ReasoningState::Idle {
            self.reasoning = ReasoningState::Started;
        }
    }

    fn content_delta(&mut self, delta: &mut StreamDelta, mut content: String) {
        if self.reasoning == ReasoningState::Started {
            self.reasoning = ReasoningState::Finished;
        }
        if content.contains(THINK_OPEN) {
            content = content.replace(THINK_OPEN, "");
            self.start_reasoning();
        }
        if self.reasoning != ReasoningState::Idle && content.contains(THINK_CLOSE) {
            content = content.replace(THINK_CLOSE, "");
            self.reasoning = ReasoningState::Finished;
        }
        if self.reasoning == ReasoningState::Started {
            delta.reasoning_content = Some(content);
        } else {
            delta.content = Some(content);
        }
    }

    fn finish_reason(&self, raw: &RawChunk) -> String {
        if self.seen_tool_calls {
            return String::from(TOOL_CALLS_FINISH_REASON);
        }
        match raw.done_reason.as_deref() {
            Some(reason) if !reason.is_empty() => reason.to_owned(),
            Some(_) | None => String::from(STOP_FINISH_REASON),
        }
    }
}

// LiteLLM's _is_function_call_complete: an object is complete, a string is complete when it parses as JSON
#[must_use]
pub fn is_function_call_complete(arguments: &Json) -> bool {
    match arguments {
        Json::Object(_) => true,
        Json::Text(text) => serde_json::from_str::<Json>(text).is_ok(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => false,
    }
}

fn done_chunk_usage(chunk: &RawChunk) -> Option<Usage> {
    if !chunk.done {
        return None;
    }
    let prompt_tokens = chunk.prompt_eval_count?;
    let completion_tokens = chunk.eval_count?;
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    })
}

fn delta_tool_calls(
    calls: &[RawToolCall],
    ids: &dyn IdSource,
) -> Result<Vec<DeltaToolCall>, OllamaError> {
    let mut converted = Vec::with_capacity(calls.len());
    let mut current_index: u32 = 0;
    for call in calls {
        let index = call.index.unwrap_or_else(|| {
            let assigned = current_index;
            current_index = current_index.saturating_add(1);
            assigned
        });
        let complete = arguments_have_length(&call.function)?
            && call
                .function
                .arguments
                .as_ref()
                .is_some_and(is_function_call_complete);
        let id = if complete {
            Some(ids.next_id())
        } else {
            call.id.clone()
        };
        converted.push(DeltaToolCall {
            id,
            kind: call
                .kind
                .clone()
                .unwrap_or_else(|| String::from(TOOL_CALL_TYPE)),
            index,
            function: delta_function(&call.function)?,
        });
    }
    Ok(converted)
}

fn arguments_have_length(function: &RawFunction) -> Result<bool, OllamaError> {
    match &function.arguments {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Object(fields)) => Ok(!fields.is_empty()),
        Some(Json::Text(text)) => Ok(!text.is_empty()),
        Some(Json::List(items)) => Ok(!items.is_empty()),
        Some(Json::Bool(_) | Json::Number(_)) => Err(OllamaError::ChunkField {
            field: "message.tool_calls.function.arguments",
        }),
    }
}

fn delta_function(function: &RawFunction) -> Result<DeltaFunction, OllamaError> {
    let arguments = match &function.arguments {
        Some(value @ Json::Object(_)) => {
            serde_json::to_string(value).map_err(|source| OllamaError::Shape { source })?
        }
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => parameters_arguments(function)?,
        Some(Json::List(_) | Json::Bool(_) | Json::Number(_)) => {
            return Err(OllamaError::ChunkField {
                field: "message.tool_calls.function.arguments",
            });
        }
    };
    Ok(DeltaFunction {
        name: function.name.clone(),
        arguments,
    })
}

fn parameters_arguments(function: &RawFunction) -> Result<String, OllamaError> {
    match &function.parameters {
        Some(value @ Json::Object(_)) => {
            serde_json::to_string(value).map_err(|source| OllamaError::Shape { source })
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Ok(String::new()),
    }
}
