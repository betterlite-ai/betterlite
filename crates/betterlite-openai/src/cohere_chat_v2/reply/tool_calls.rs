use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::CohereV2Error;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReplyToolCall {
    pub index: usize,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ReplyFunction,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReplyFunction {
    pub name: Option<String>,
    pub arguments: String,
}

pub(super) fn field_tool_calls(
    fields: &Object,
) -> Result<Option<Vec<ReplyToolCall>>, CohereV2Error> {
    match fields.get("tool_calls") {
        Some(Json::List(entries)) if !entries.is_empty() => Ok(Some(tool_calls(entries)?)),
        Some(Json::Null | Json::List(_)) | None => Ok(None),
        Some(_) => Err(CohereV2Error::NotAList {
            field: "message.tool_calls",
        }),
    }
}

fn tool_calls(entries: &[Json]) -> Result<Vec<ReplyToolCall>, CohereV2Error> {
    let mut calls = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        calls.push(tool_call(index, entry)?);
    }
    Ok(calls)
}

fn tool_call(index: usize, entry: &Json) -> Result<ReplyToolCall, CohereV2Error> {
    let Json::Object(tool) = entry else {
        return Err(CohereV2Error::NotAnObject {
            field: "message.tool_calls",
        });
    };
    Ok(ReplyToolCall {
        index,
        id: tool_id(tool)?,
        kind: tool_kind(tool)?,
        function: tool_function(tool)?,
    })
}

fn tool_id(tool: &Object) -> Result<String, CohereV2Error> {
    match tool.get("id") {
        Some(Json::Text(id)) => Ok(id.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Err(CohereV2Error::NotText {
            field: "message.tool_calls.id",
        }),
    }
}

fn tool_kind(tool: &Object) -> Result<String, CohereV2Error> {
    match tool.get("type") {
        None | Some(Json::Null) => Ok(String::from("function")),
        Some(Json::Text(kind)) => Ok(kind.clone()),
        Some(_) => Err(CohereV2Error::NotText {
            field: "message.tool_calls.type",
        }),
    }
}

fn tool_function(tool: &Object) -> Result<ReplyFunction, CohereV2Error> {
    match tool.get("function") {
        Some(Json::Object(fields)) => function(fields),
        Some(_) => Err(CohereV2Error::NotAnObject {
            field: "message.tool_calls.function",
        }),
        None => Err(CohereV2Error::Missing {
            field: "message.tool_calls.function",
        }),
    }
}

fn function(fields: &Object) -> Result<ReplyFunction, CohereV2Error> {
    let name = match fields.get("name") {
        None | Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name.clone()),
        Some(_) => {
            return Err(CohereV2Error::NotText {
                field: "message.tool_calls.function.name",
            });
        }
    };
    let arguments = match fields.get("parameters") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Object(parameters)) => {
            serde_json::to_string(parameters).map_err(|source| CohereV2Error::Encode { source })?
        }
        Some(_) => {
            return Err(CohereV2Error::NotAnObject {
                field: "message.tool_calls.function.parameters",
            });
        }
    };
    Ok(ReplyFunction { name, arguments })
}
