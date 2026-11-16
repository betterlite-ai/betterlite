use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use crate::databricks_chat::content::DatabricksContent;
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::function::Arguments;
use crate::databricks_chat::keyed::Keyed;
use crate::databricks_chat::stream::ChunkParser;
use crate::databricks_chat::stream::delta::{Delta, build_delta};
use crate::databricks_chat::stream::wire::{ChunkChoice, DatabricksDelta};

const JSON_TOOL_CALL: &str = "json_tool_call";

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

pub(super) fn transform(
    choice: ChunkChoice,
    parser: &mut ChunkParser,
    chunk: &str,
) -> Result<StreamChoice, DatabricksChatError> {
    let Some(mut delta) = choice.delta.required("delta", chunk)? else {
        return Err(DatabricksChatError::Shape {
            message: String::from("the delta of a Databricks stream choice is null"),
        });
    };
    prepare_tool_calls(&mut delta, parser, chunk)?;
    add_citation(&mut delta);
    let mut extra = choice.extra;
    extra.shift_remove("message");
    Ok(StreamChoice {
        finish_reason: choice
            .finish_reason
            .filter(|reason| !reason.is_empty())
            .map(|reason| choice_finish_reason(Some(reason.as_str())).0),
        index: match choice.index {
            Keyed::Given(given) => Some(given),
            Keyed::Null => None,
            Keyed::Missing => Some(Number::from(0_i64)),
        },
        delta: build_delta(delta)?,
        logprobs: choice.logprobs,
        extra,
    })
}

// The json-mode conversion of a json_tool_call, or outside json mode the "{}" arguments cleanup
fn prepare_tool_calls(
    delta: &mut DatabricksDelta,
    parser: &mut ChunkParser,
    chunk: &str,
) -> Result<(), DatabricksChatError> {
    let Some(calls) = delta.tool_calls.as_mut() else {
        return Ok(());
    };
    if !parser.json_mode {
        for call in calls.iter_mut() {
            if let Some(function) = &mut call.function
                && let Some(Arguments::Text(text)) = &mut function.arguments
                && text.as_str() == "{}"
            {
                text.clear();
            }
        }
        return Ok(());
    }
    let Some(first) = calls.first() else {
        return Ok(());
    };
    if let Some(name) = first
        .function
        .as_ref()
        .and_then(|function| function.name.clone())
    {
        parser.last_function_name = Some(name);
    }
    if parser.last_function_name.as_deref() != Some(JSON_TOOL_CALL) {
        return Ok(());
    }
    let Some(function) = &first.function else {
        return Err(DatabricksChatError::ChunkKey {
            key: "function",
            chunk: chunk.to_owned(),
        });
    };
    let content = match &function.arguments {
        None => return Ok(()),
        Some(Arguments::Text(raw)) => converted_arguments(raw)?,
        Some(Arguments::Encoded(_)) => {
            return Err(DatabricksChatError::Shape {
                message: String::from("the arguments of a json mode tool call must be a string"),
            });
        }
    };
    delta.content = Some(DatabricksContent::Text(content));
    delta.tool_calls = None;
    Ok(())
}

// _convert_tool_response_to_message: json.loads, the values key when set, and "{}" becomes ""
fn converted_arguments(raw: &str) -> Result<String, DatabricksChatError> {
    let Ok(parsed) = serde_json::from_str::<Json>(raw) else {
        return Ok(raw.to_owned());
    };
    let values = match &parsed {
        Json::Object(fields) => fields
            .get("values")
            .filter(|values| !matches!(values, Json::Null)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    let content =
        serde_json::to_string(values.unwrap_or(&parsed)).map_err(DatabricksChatError::Encode)?;
    if content == "{}" {
        return Ok(String::new());
    }
    Ok(content)
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
