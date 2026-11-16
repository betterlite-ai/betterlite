use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json as EncodedJson;
use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use crate::databricks_chat::content::{Arguments, DatabricksContent};
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::Usage;
use crate::databricks_chat::response::usage::usage_from;
use crate::databricks_chat::slot::Slot;
use crate::databricks_chat::stream::delta::{Delta, build_delta};
use crate::databricks_chat::stream::wire::{ChunkChoice, DatabricksChunk, DatabricksDelta};
use crate::ollama_chat::ids::IdSource;

pub mod delta;
mod tests;
pub mod wire;

const JSON_TOOL_CALL: &str = "json_tool_call";

// ModelResponseStream as DatabricksChatResponseIterator builds it for one chunk
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamChunk {
    pub id: String,
    pub created: i64,
    pub model: Option<String>,
    pub object: String,
    pub choices: Vec<StreamChoice>,
    pub provider_specific_fields: Option<Object>,
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

// StreamingChoices: the finish reason maps, index and logprobs pass through, extra keys stay
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamChoice {
    pub finish_reason: Option<String>,
    pub index: Option<Number>,
    pub delta: Delta,
    pub logprobs: Option<Json>,
    #[serde(flatten)]
    pub extra: Object,
}

// DatabricksChatResponseIterator: the chunk parser keeps the last function name it saw in json mode
#[derive(Debug, Default)]
pub struct ChunkParser {
    pub json_mode: bool,
    pub last_function_name: Option<String>,
}

impl ChunkParser {
    // chunk_parser over one decoded data line, reading the choices first as LiteLLM does
    pub fn parse(
        &mut self,
        chunk: &str,
        ids: &dyn IdSource,
    ) -> Result<StreamChunk, DatabricksChatError> {
        let parsed = decode_chunk(chunk)?;
        let choices = chunk_key(parsed.choices, "choices", chunk)?.ok_or_else(|| {
            DatabricksChatError::Shape {
                message: String::from("the choices of a Databricks stream chunk are null"),
            }
        })?;
        let mut transformed = Vec::with_capacity(choices.len());
        for choice in choices {
            transformed.push(transform_choice(choice, self, chunk)?);
        }
        let id = chunk_key(parsed.id, "id", chunk)?.unwrap_or_else(|| ids.next_id());
        let created =
            chunk_key(parsed.created, "created", chunk)?.ok_or(DatabricksChatError::NullCreated)?;
        let model = chunk_key(parsed.model, "model", chunk)?;
        let service_tier = match parsed.service_tier {
            Some(Json::Text(tier)) if !tier.is_empty() => Some(tier),
            None | Some(_) => None,
        };
        Ok(StreamChunk {
            id,
            created,
            model,
            object: String::from("chat.completion.chunk"),
            choices: transformed,
            provider_specific_fields: None,
            usage: parsed.usage.as_ref().map(usage_from).transpose()?,
            service_tier,
        })
    }
}

// A JSON array would otherwise decode as an all-default chunk and report a missing key
fn decode_chunk(chunk: &str) -> Result<DatabricksChunk, DatabricksChatError> {
    if chunk.trim_start().starts_with('[') {
        return Err(DatabricksChatError::Shape {
            message: String::from("a Databricks stream chunk must be a JSON object"),
        });
    }
    serde_json::from_str(chunk).map_err(DatabricksChatError::Decode)
}

// A key chunk_parser reads with []: absent is the KeyError, null is None for the caller to judge
fn chunk_key<Item>(
    slot: Slot<Item>,
    key: &'static str,
    chunk: &str,
) -> Result<Option<Item>, DatabricksChatError> {
    match slot {
        Slot::Value(item) => Ok(Some(item)),
        Slot::Null => Ok(None),
        Slot::Absent => Err(DatabricksChatError::ChunkKey {
            key,
            chunk: chunk.to_owned(),
        }),
    }
}

// One choice: the json mode rewrite, the "{}" cleanup, the citation, then Delta(**delta)
fn transform_choice(
    given: ChunkChoice,
    parser: &mut ChunkParser,
    chunk: &str,
) -> Result<StreamChoice, DatabricksChatError> {
    let mut delta =
        chunk_key(given.delta, "delta", chunk)?.ok_or_else(|| DatabricksChatError::Shape {
            message: String::from("the delta of a Databricks stream choice is null"),
        })?;
    if parser.json_mode {
        convert_json_tool_call(&mut delta, parser, chunk)?;
    } else {
        clear_empty_arguments(&mut delta)?;
    }
    add_citation(&mut delta)?;
    let index = match given.index {
        Slot::Value(index) => Some(index),
        Slot::Null => None,
        Slot::Absent => Some(Number::from(0_i64)),
    };
    let mut extra = given.extra;
    extra.shift_remove("message");
    Ok(StreamChoice {
        finish_reason: given
            .finish_reason
            .filter(|reason| !reason.is_empty())
            .map(|reason| choice_finish_reason(Some(reason.as_str())).0),
        index,
        delta: build_delta(delta)?,
        logprobs: given.logprobs,
        extra,
    })
}

// Outside json mode an arguments value of exactly "{}" becomes "", so the caller never sees invalid json
fn clear_empty_arguments(delta: &mut DatabricksDelta) -> Result<(), DatabricksChatError> {
    for call in delta.tool_calls.iter_mut().flatten() {
        let function = match &mut call.function {
            Slot::Value(function) => function,
            Slot::Null => {
                return Err(DatabricksChatError::Shape {
                    message: String::from("the function of a Databricks stream tool call is null"),
                });
            }
            Slot::Absent => continue,
        };
        if let Some(Arguments::Text(text)) = &mut function.arguments
            && text.as_str() == "{}"
        {
            text.clear();
        }
    }
    Ok(())
}

// In json mode the json_tool_call arguments become the delta content and the tool calls go away
fn convert_json_tool_call(
    delta: &mut DatabricksDelta,
    parser: &mut ChunkParser,
    chunk: &str,
) -> Result<(), DatabricksChatError> {
    let Some(first) = delta.tool_calls.as_ref().and_then(|calls| calls.first()) else {
        return Ok(());
    };
    let called = match &first.function {
        Slot::Value(function) => Some(function),
        Slot::Null => {
            return Err(DatabricksChatError::Shape {
                message: String::from("the function of a Databricks stream tool call is null"),
            });
        }
        Slot::Absent => None,
    };
    if let Some(name) = called.and_then(|function| function.name.clone()) {
        parser.last_function_name = Some(name);
    }
    if parser.last_function_name.as_deref() != Some(JSON_TOOL_CALL) {
        return Ok(());
    }
    let Some(function) = called else {
        return Err(DatabricksChatError::ChunkKey {
            key: "function",
            chunk: chunk.to_owned(),
        });
    };
    let content = match &function.arguments {
        Some(Arguments::Text(raw)) => converted_arguments(raw),
        Some(Arguments::Encoded(_)) => {
            return Err(DatabricksChatError::Shape {
                message: String::from("the arguments of a json mode tool call must be a string"),
            });
        }
        None => return Ok(()),
    };
    delta.content = Some(DatabricksContent::Text(content));
    delta.tool_calls = None;
    Ok(())
}

// _convert_tool_response_to_message: json.loads, the values key when set, and "{}" becomes ""
fn converted_arguments(raw: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<EncodedJson>(raw) else {
        return raw.to_owned();
    };
    let values = match &parsed {
        EncodedJson::Object(fields) => fields
            .get("values")
            .filter(|values| !matches!(values, EncodedJson::Null)),
        EncodedJson::Null
        | EncodedJson::Bool(_)
        | EncodedJson::Number(_)
        | EncodedJson::Text(_)
        | EncodedJson::List(_) => None,
    };
    let content = dumps(values.unwrap_or(&parsed));
    if content == "{}" {
        return String::new();
    }
    content
}

// The first citation of the first content block goes to provider_specific_fields.citation
fn add_citation(delta: &mut DatabricksDelta) -> Result<(), DatabricksChatError> {
    let first_citation = match &delta.content {
        Some(DatabricksContent::Blocks(blocks)) => blocks
            .first()
            .and_then(|block| block.citations.as_ref())
            .and_then(|citations| citations.first())
            .cloned(),
        Some(DatabricksContent::Text(_)) | None => None,
    };
    let Some(citation) = first_citation else {
        return Ok(());
    };
    match &mut delta.provider_specific_fields {
        Slot::Absent => {
            delta.provider_specific_fields =
                Slot::Value(Object::from_iter([(String::from("citation"), citation)]));
        }
        Slot::Value(fields) => {
            fields.insert(String::from("citation"), citation);
        }
        Slot::Null => {
            return Err(DatabricksChatError::Shape {
                message: String::from(
                    "a delta with a citation cannot take a null provider_specific_fields",
                ),
            });
        }
    }
    Ok(())
}
