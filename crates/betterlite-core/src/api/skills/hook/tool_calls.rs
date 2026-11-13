use betterlite_openai::chat::response::ChatChoice;
use serde_json::{Map, Value};
use thiserror::Error;

use super::response::{ContentBlock, HookResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedToolCall {
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Value,
}

#[derive(Debug, Error)]
pub enum ToolCallError {
    #[error("the arguments of tool call {name} are not valid JSON: {source}")]
    Arguments {
        name: String,
        #[source]
        source: serde_json::Error,
    },
}

pub fn extract_tool_calls(
    response: &HookResponse,
) -> Result<Vec<ExtractedToolCall>, ToolCallError> {
    match response {
        HookResponse::Dict(fields) => Ok(extract_dict_tool_calls(fields)),
        HookResponse::Object(object) => {
            let mut tool_calls = extract_block_tool_calls(&object.content);
            if tool_calls.is_empty() {
                tool_calls = extract_choice_tool_calls(&object.choices)?;
            }
            Ok(tool_calls)
        }
    }
}

fn extract_dict_tool_calls(fields: &Map<String, Value>) -> Vec<ExtractedToolCall> {
    let Some(Value::Array(blocks)) = fields.get("content") else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| {
            let Value::Object(tool_block) = block else {
                return None;
            };
            if tool_block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return None;
            }
            Some(ExtractedToolCall {
                id: tool_block
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                name: tool_block
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                input: tool_input(tool_block.get("input")),
            })
        })
        .collect()
}

fn extract_block_tool_calls(blocks: &[ContentBlock]) -> Vec<ExtractedToolCall> {
    blocks
        .iter()
        .filter(|block| block.kind == "tool_use")
        .map(|block| ExtractedToolCall {
            id: block.id.clone(),
            name: block.name.clone(),
            input: tool_input(Some(&block.input)),
        })
        .collect()
}

fn extract_choice_tool_calls(
    choices: &[ChatChoice],
) -> Result<Vec<ExtractedToolCall>, ToolCallError> {
    let Some(choice) = choices.first() else {
        return Ok(Vec::new());
    };
    let Some(tool_calls) = choice.message.tool_calls.as_ref() else {
        return Ok(Vec::new());
    };
    let mut extracted = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        let arguments = &tool_call.function.arguments;
        let input = if arguments.is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(arguments).map_err(|source| ToolCallError::Arguments {
                name: tool_call.function.name.clone(),
                source,
            })?
        };
        extracted.push(ExtractedToolCall {
            id: Some(tool_call.id.clone()),
            name: Some(tool_call.function.name.clone()),
            input,
        });
    }
    Ok(extracted)
}

fn tool_input(input: Option<&Value>) -> Value {
    match input {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(present) => present.clone(),
    }
}
