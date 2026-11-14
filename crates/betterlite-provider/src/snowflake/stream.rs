use std::convert::Infallible;
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

#[derive(Debug, Error)]
pub enum StreamError<CounterFailure = Infallible>
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
    #[error("{0}")]
    Message(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SnowflakeChunk {
    OpenAi(OpenAiStreamChunk),
    Anthropic(StreamEventChunk),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
            .is_some_and(|fields| fields.contains_key("choices"));
        if has_choices {
            return Ok(SnowflakeChunk::OpenAi(parse_openai_chunk(chunk)?));
        }
        Ok(SnowflakeChunk::Anthropic(self.parser.chunk_parser(chunk)?))
    }
}

fn parse_openai_chunk(chunk: &Json) -> Result<OpenAiStreamChunk, OpenAiChunkError> {
    let fields = chunk.as_object().ok_or(OpenAiChunkError::Message(
        "a stream chunk must be an object",
    ))?;
    let choices = match fields.get("choices") {
        None | Some(Json::Null) => return Ok(OpenAiStreamChunk::default()),
        Some(Json::List(items)) => items.as_slice(),
        Some(_) => {
            return Err(OpenAiChunkError::Message(
                "the stream chunk choices must be a list",
            ));
        }
    };
    let Some(first) = choices.first() else {
        return Ok(OpenAiStreamChunk::default());
    };
    let choice = first
        .as_object()
        .ok_or(OpenAiChunkError::Message("a choice must be an object"))?;
    let empty_delta = Object::new();
    let delta = match choice.get("delta") {
        None | Some(Json::Null) => &empty_delta,
        Some(Json::Object(delta)) => delta,
        Some(_) => return Err(OpenAiChunkError::Message("delta must be an object")),
    };
    let finish_reason = text_field(
        choice.get("finish_reason"),
        "finish_reason must be a string",
    )?;
    Ok(OpenAiStreamChunk {
        text: text_field(
            delta.get("content"),
            "the stream chunk field content must be a string",
        )?,
        is_finished: !finish_reason.is_empty(),
        finish_reason,
        usage: None,
        index: index_field(choice.get("index"))?,
        tool_use: tool_use(delta.get("tool_calls"))?,
    })
}

fn tool_use(tool_calls: Option<&Json>) -> Result<Option<StreamToolUse>, OpenAiChunkError> {
    let Some(Json::List(calls)) = tool_calls else {
        return match tool_calls {
            None | Some(Json::Null) => Ok(None),
            Some(_) => Err(OpenAiChunkError::Message("tool_calls must be a list")),
        };
    };
    let Some(first) = calls.first() else {
        return Ok(None);
    };
    let call = first
        .as_object()
        .ok_or(OpenAiChunkError::Message("a tool call must be an object"))?;
    let empty_function = Object::new();
    let function = match call.get("function") {
        None | Some(Json::Null) => &empty_function,
        Some(Json::Object(function)) => function,
        Some(_) => return Err(OpenAiChunkError::Message("function must be an object")),
    };
    Ok(Some(StreamToolUse {
        id: text_field(call.get("id"), "id must be a string")?,
        kind: "function".to_owned(),
        function: StreamToolFunction {
            name: text_field(function.get("name"), "name must be a string")?,
            arguments: text_field(function.get("arguments"), "arguments must be a string")?,
        },
        index: index_field(call.get("index"))?,
    }))
}

// LiteLLM reads "value or ''", so an absent or null value reads as the empty string and another type is an error
fn text_field(value: Option<&Json>, message: &'static str) -> Result<String, OpenAiChunkError> {
    match value {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(OpenAiChunkError::Message(message)),
    }
}

fn index_field(value: Option<&Json>) -> Result<i64, OpenAiChunkError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(OpenAiChunkError::Message("index must be an integer")),
        Some(_) => Err(OpenAiChunkError::Message("index must be an integer")),
    }
}

#[cfg(test)]
mod tests;
