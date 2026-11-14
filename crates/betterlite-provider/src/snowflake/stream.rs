use std::error::Error as StdError;

use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_anthropic::stream_state::state::StreamState as AnthropicState;
use betterlite_anthropic::stream_state::state::{IdSource, StreamOptions};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_context::provider_message_primitives::StreamState as BaseStreamState;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

const CHOICES: &str = "choices";
const DELTA: &str = "delta";
const CONTENT: &str = "content";
const FINISH_REASON: &str = "finish_reason";
const TOOL_CALLS: &str = "tool_calls";
const FUNCTION: &str = "function";
const NAME: &str = "name";
const ARGUMENTS: &str = "arguments";
const INDEX: &str = "index";
const ID: &str = "id";

#[derive(Debug, Error)]
pub enum StreamError<CounterFailure = std::convert::Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    OpenAi(#[from] OpenAiChunkError),
    #[error(transparent)]
    Anthropic(#[from] StreamEventError<CounterFailure>),
}

#[derive(Debug, Error)]
pub enum OpenAiChunkError {
    #[error("the stream chunk choices must be a list")]
    Choices,
    #[error("a stream chunk choice must be an object")]
    Choice,
    #[error("a stream chunk tool call must be an object")]
    ToolCall,
    #[error("the stream chunk field {field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SnowflakeChunk {
    OpenAi(OpenAiStreamChunk),
    Anthropic(StreamEventChunk),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiStreamChunk {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<Json>,
    pub index: i64,
    pub tool_use: Option<StreamToolUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamToolUse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: StreamToolFunction,
    pub index: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamToolFunction {
    pub name: String,
    pub arguments: String,
}

pub struct SnowflakeStreamingHandler<Stream, Response, Counter> {
    // B2214's base iterator, over the same stream the Anthropic parser reads
    pub base: BaseStreamState<Stream, Response>,
    pub parser: StreamEventConverter<Counter>,
}

// LiteLLM's get_model_response_iterator wires the base iterator and the Anthropic parser to one stream
#[must_use]
pub fn get_model_response_iterator<Stream, Response, Counter>(
    stream: Stream,
    sync_stream: bool,
    json_mode: Option<bool>,
    counter: Counter,
    ids: &dyn IdSource,
) -> SnowflakeStreamingHandler<Stream, Response, Counter>
where
    Counter: ReasoningTokenCounter,
{
    let options = StreamOptions {
        sync_stream,
        json_mode,
        speed: None,
        tool_name_reverse_map: IndexMap::new(),
    };
    SnowflakeStreamingHandler {
        base: BaseStreamState {
            stream,
            json_mode,
            http_response: None,
        },
        parser: StreamEventConverter {
            state: AnthropicState::new(options, ids),
            counter,
            json_could_close: false,
        },
    }
}

impl<Stream, Response, Counter> SnowflakeStreamingHandler<Stream, Response, Counter>
where
    Counter: ReasoningTokenCounter,
{
    pub fn chunk_parser(
        &mut self,
        chunk: &Json,
    ) -> Result<SnowflakeChunk, StreamError<Counter::Failure>> {
        let has_choices = chunk
            .as_object()
            .is_some_and(|fields| fields.contains_key(CHOICES));
        if has_choices {
            return Ok(SnowflakeChunk::OpenAi(parse_openai_chunk(chunk)?));
        }
        Ok(SnowflakeChunk::Anthropic(self.parser.chunk_parser(chunk)?))
    }
}

fn parse_openai_chunk(chunk: &Json) -> Result<OpenAiStreamChunk, OpenAiChunkError> {
    let fields = chunk.as_object().ok_or(OpenAiChunkError::Choice)?;
    let choices = match fields.get(CHOICES) {
        None => &[][..],
        Some(Json::List(items)) => items.as_slice(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(OpenAiChunkError::Choices);
        }
    };
    let Some(choice) = choices.first() else {
        return Ok(empty_chunk());
    };
    let choice = choice.as_object().ok_or(OpenAiChunkError::Choice)?;
    let empty_delta = Object::new();
    let delta = match choice.get(DELTA) {
        None | Some(Json::Null) => &empty_delta,
        Some(Json::Object(delta)) => delta,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(OpenAiChunkError::Field {
                field: DELTA,
                expected: "an object",
            });
        }
    };
    let finish_reason = text_field(choice.get(FINISH_REASON), FINISH_REASON)?;
    Ok(OpenAiStreamChunk {
        text: text_field(delta.get(CONTENT), CONTENT)?,
        is_finished: !finish_reason.is_empty(),
        finish_reason,
        usage: None,
        index: index_field(choice.get(INDEX))?,
        tool_use: tool_use(delta.get(TOOL_CALLS))?,
    })
}

// LiteLLM answers an empty choices list with the same empty chunk it would build for no delta at all
const fn empty_chunk() -> OpenAiStreamChunk {
    OpenAiStreamChunk {
        text: String::new(),
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index: 0,
        tool_use: None,
    }
}

fn tool_use(tool_calls: Option<&Json>) -> Result<Option<StreamToolUse>, OpenAiChunkError> {
    let Some(Json::List(calls)) = tool_calls else {
        return match tool_calls {
            None | Some(Json::Null | Json::List(_)) => Ok(None),
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                Err(OpenAiChunkError::Field {
                    field: TOOL_CALLS,
                    expected: "a list",
                })
            }
        };
    };
    let Some(call) = calls.first() else {
        return Ok(None);
    };
    let call = call.as_object().ok_or(OpenAiChunkError::ToolCall)?;
    let empty_function = Object::new();
    let function = match call.get(FUNCTION) {
        None | Some(Json::Null) => &empty_function,
        Some(Json::Object(function)) => function,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(OpenAiChunkError::Field {
                field: FUNCTION,
                expected: "an object",
            });
        }
    };
    Ok(Some(StreamToolUse {
        id: text_field(call.get(ID), ID)?,
        kind: "function".to_owned(),
        function: StreamToolFunction {
            name: text_field(function.get(NAME), NAME)?,
            arguments: text_field(function.get(ARGUMENTS), ARGUMENTS)?,
        },
        index: index_field(call.get(INDEX))?,
    }))
}

// LiteLLM reads "value or ''", so an absent or null value reads as the empty string and another type is an error
fn text_field(value: Option<&Json>, field: &'static str) -> Result<String, OpenAiChunkError> {
    match value {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(OpenAiChunkError::Field {
                field,
                expected: "a string",
            })
        }
    }
}

fn index_field(value: Option<&Json>) -> Result<i64, OpenAiChunkError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(OpenAiChunkError::Field {
            field: INDEX,
            expected: "an integer",
        }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(OpenAiChunkError::Field {
                field: INDEX,
                expected: "an integer",
            })
        }
    }
}

#[cfg(test)]
mod tests;
