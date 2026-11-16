use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json as EncodedJson;
use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use crate::databricks_chat::content::{
    Arguments, DatabricksContent, extract_content_str, extract_reasoning,
};
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::Usage;
use crate::databricks_chat::response::usage::usage_from;
use crate::databricks_chat::stream::delta::{Delta, build_delta};
use crate::databricks_chat::stream::wire::{ChunkChoice, DatabricksChunk, DatabricksDelta};
use crate::ollama_chat::ids::IdSource;

pub mod delta;
mod tests;
pub mod wire;

const JSON_TOOL_CALL: &str = "json_tool_call";

// ModelResponseStream as DatabricksChatResponseIterator builds it for one chunk
#[derive(Debug, Clone, PartialEq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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
        let parsed: DatabricksChunk =
            serde_json::from_str(chunk).map_err(DatabricksChatError::Decode)?;
        let choices = match parsed.choices {
            Some(Some(choices)) => choices,
            Some(None) => {
                return Err(DatabricksChatError::Shape {
                    message: String::from("the choices of a Databricks stream chunk are null"),
                });
            }
            None => {
                return Err(DatabricksChatError::ChunkKey {
                    key: "choices",
                    chunk: chunk.to_owned(),
                });
            }
        };
        let mut transformed = Vec::with_capacity(choices.len());
        for choice in choices {
            transformed.push(transform_choice(choice, self, chunk)?);
        }
        let id = match parsed.id {
            Some(Some(id)) => id,
            Some(None) => ids.next_id(),
            None => {
                return Err(DatabricksChatError::ChunkKey {
                    key: "id",
                    chunk: chunk.to_owned(),
                });
            }
        };
        let created = match parsed.created {
            Some(Some(created)) => created,
            Some(None) => return Err(DatabricksChatError::NullCreated),
            None => {
                return Err(DatabricksChatError::ChunkKey {
                    key: "created",
                    chunk: chunk.to_owned(),
                });
            }
        };
        let model = match parsed.model {
            Some(Some(model)) => Some(model),
            Some(None) => None,
            None => {
                return Err(DatabricksChatError::ChunkKey {
                    key: "model",
                    chunk: chunk.to_owned(),
                });
            }
        };
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

// One choice: the json mode rewrite, the "{}" cleanup, the citation, then Delta(**delta)
fn transform_choice(
    given: ChunkChoice,
    parser: &mut ChunkParser,
    chunk: &str,
) -> Result<StreamChoice, DatabricksChatError> {
    let mut delta = match given.delta {
        Some(Some(delta)) => delta,
        Some(None) => {
            return Err(DatabricksChatError::Shape {
                message: String::from("the delta of a Databricks stream choice is null"),
            });
        }
        None => {
            return Err(DatabricksChatError::ChunkKey {
                key: "delta",
                chunk: chunk.to_owned(),
            });
        }
    };
    if parser.json_mode {
        convert_json_tool_call(&mut delta, parser, chunk)?;
    } else {
        clear_empty_arguments(&mut delta)?;
    }
    add_citation(&mut delta);
    let index = match given.index {
        Some(Some(index)) => Some(index),
        Some(None) => None,
        None => Some(Number::from(0_i64)),
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
            Some(Some(function)) => function,
            Some(None) => {
                return Err(DatabricksChatError::Shape {
                    message: String::from("the function of a Databricks stream tool call is null"),
                });
            }
            None => continue,
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
    let function = match &first.function {
        Some(Some(function)) => Some(function),
        Some(None) => {
            return Err(DatabricksChatError::Shape {
                message: String::from("the function of a Databricks stream tool call is null"),
            });
        }
        None => None,
    };
    if let Some(name) = function.and_then(|function| function.name.clone()) {
        parser.last_function_name = Some(name);
    }
    if parser.last_function_name.as_deref() != Some(JSON_TOOL_CALL) {
        return Ok(());
    }
    let Some(function) = function else {
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
fn add_citation(delta: &mut DatabricksDelta) {
    let first_citation = match &delta.content {
        Some(DatabricksContent::Blocks(blocks)) => blocks
            .first()
            .and_then(|block| block.citations.as_ref())
            .and_then(|citations| citations.first())
            .cloned(),
        Some(DatabricksContent::Text(_)) | None => None,
    };
    if let Some(citation) = first_citation {
        delta
            .provider_specific_fields
            .get_or_insert_with(Object::new)
            .insert(String::from("citation"), citation);
    }
}
