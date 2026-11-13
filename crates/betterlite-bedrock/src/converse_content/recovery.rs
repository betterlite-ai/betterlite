use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ToolBlock, decode_converse_json_tool, get_tool_call_names};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AssistantMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<MessageToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessageToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: MessageFunction,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessageFunction {
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolCallRecovery {
    pub message: AssistantMessage,
    pub finish_reason: Option<String>,
}

#[must_use]
pub fn recover_tool_call(
    message: &mut AssistantMessage,
    tools: Option<&[ToolBlock]>,
    initial_finish_reason: Option<&str>,
    identifier: &dyn Fn() -> Uuid,
) -> Option<String> {
    let unchanged = initial_finish_reason.map(str::to_owned);
    let Some(known_tools) = tools else {
        return unchanged;
    };
    let Some(content) = message.content.as_deref() else {
        return unchanged;
    };
    let Ok(Json::Object(fields)) = decode_converse_json_tool(content.as_bytes()) else {
        return unchanged;
    };
    let Some(Json::Text(kind)) = fields.get("type") else {
        return unchanged;
    };
    let Some(Json::Text(name)) = fields.get("name") else {
        return unchanged;
    };
    if kind != "function"
        || !get_tool_call_names(Some(known_tools))
            .iter()
            .any(|known| known == name)
    {
        return unchanged;
    }
    let Some(arguments) = recovered_arguments(&fields) else {
        return unchanged;
    };
    message.tool_calls = Some(vec![MessageToolCall {
        id: identifier().to_string(),
        kind: "function".to_owned(),
        function: MessageFunction {
            name: Some(name.clone()),
            arguments,
        },
    }]);
    message.content = None;
    Some("tool_calls".to_owned())
}

// Python's Function(**json_content) stringifies dict arguments and leaves a missing one empty.
fn recovered_arguments(fields: &IndexMap<String, Json>) -> Option<String> {
    match fields.get("arguments") {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Object(arguments)) => Some(dumps_object(arguments)),
        Some(Json::Null) | None => Some(match fields.get("parameters") {
            Some(Json::Object(parameters)) => dumps_object(parameters),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => String::new(),
        }),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => None,
    }
}
