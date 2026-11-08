use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::ReplyError;

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

pub(super) fn field_tool_calls(fields: &Object) -> Result<Option<Vec<ReplyToolCall>>, ReplyError> {
    match fields.get("tool_calls") {
        Some(Json::List(entries)) if !entries.is_empty() => Ok(Some(tool_calls(entries)?)),
        Some(Json::Null | Json::List(_)) | None => Ok(None),
        Some(_) => Err(ReplyError::Field("message.tool_calls", "is not a list")),
    }
}

fn tool_calls(entries: &[Json]) -> Result<Vec<ReplyToolCall>, ReplyError> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| tool_call(index, entry))
        .collect()
}

fn tool_call(index: usize, entry: &Json) -> Result<ReplyToolCall, ReplyError> {
    let Json::Object(tool) = entry else {
        return Err(ReplyError::Field("message.tool_calls", "is not an object"));
    };
    let id = match tool.get("id") {
        Some(Json::Text(id)) => id.clone(),
        Some(_) | None => return Err(ReplyError::Field("tool_calls.id", "is not a string")),
    };
    let kind = match tool.get("type") {
        None | Some(Json::Null) => String::from("function"),
        Some(Json::Text(kind)) => kind.clone(),
        Some(_) => return Err(ReplyError::Field("tool_calls.type", "is not a string")),
    };
    let function = match tool.get("function") {
        Some(Json::Object(fields)) => function(fields)?,
        Some(_) => return Err(ReplyError::Field("tool function", "is not an object")),
        None => return Err(ReplyError::Field("tool function", "is missing")),
    };
    Ok(ReplyToolCall {
        index,
        id,
        kind,
        function,
    })
}

fn function(fields: &Object) -> Result<ReplyFunction, ReplyError> {
    let name = match fields.get("name") {
        None | Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name.clone()),
        Some(_) => return Err(ReplyError::Field("tool function name", "is not a string")),
    };
    let arguments = match fields.get("parameters") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Object(parameters)) => {
            serde_json::to_string(parameters).map_err(ReplyError::Encode)?
        }
        Some(_) => {
            return Err(ReplyError::Field(
                "tool function parameters",
                "is not an object",
            ));
        }
    };
    Ok(ReplyFunction { name, arguments })
}
