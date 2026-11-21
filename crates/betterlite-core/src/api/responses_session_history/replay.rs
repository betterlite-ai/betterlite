use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::api::response_synthetic_events::python_value::has_content;
use crate::api::responses_input_messages::{InputRequest, input_messages};
use crate::api::responses_session_history::cold::stored_request;
use crate::api::responses_session_history::error::SessionHistoryError;
use crate::api::responses_session_history::{ColdStorageReader, SessionHistory, SessionLog};
use crate::api::responses_tool_repair::value::{Getter, Instance, Value};

const REDACTED_BY_LITELLM: &str = "redacted-by-litellm";
const REDACTED_ARGUMENTS: &str = "{}";
// Message's constructor deletes these attributes when they arrive as None
const DROPPED_WHEN_NULL: [&str; 6] = [
    "audio",
    "images",
    "reasoning_content",
    "thinking_blocks",
    "reasoning_items",
    "annotations",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    #[default]
    Assistant,
    User,
    System,
    Tool,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct StoredFunction {
    #[serde(default)]
    pub arguments: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct StoredToolCall {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    // a custom tool call carries its input under "custom" instead
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<StoredFunction>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

// LiteLLM's Message as ModelResponse(**response) rebuilds it from the stored choice
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct StoredMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub role: Option<MessageRole>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StoredToolCall>>,
    #[serde(default)]
    pub function_call: Option<StoredFunction>,
    #[serde(default)]
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredChoice {
    #[serde(default)]
    message: Option<StoredMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredResponse {
    #[serde(default)]
    choices: Vec<StoredChoice>,
}

// a turn replays its stored input, or its messages when the input is empty, then every response choice
pub async fn replay_turn<Query, Cold, Sleep>(
    service: &SessionHistory<'_, Query, Cold, Sleep>,
    log: &SessionLog,
    history: &mut Vec<Value>,
) -> Result<(), SessionHistoryError<Cold::Error>>
where
    Query: Sync,
    Cold: ColdStorageReader + Sync,
    Sleep: Sync,
{
    let request = stored_request(service, log).await?.unwrap_or_default();
    if let Some(chosen) = replayed_input(&request) {
        let mut assembly: InputRequest = serde_json::from_value(serde_json::to_value(&request)?)?;
        assembly.input = Some(chosen);
        for message in input_messages(&assembly, true, service.limits, service.input_cache)? {
            history.push(serde_json::from_value(message)?);
        }
    }
    let Some(Json::Object(stored)) = &log.response else {
        return Ok(());
    };
    if stored.is_empty() {
        return Ok(());
    }
    let response: StoredResponse = serde_json::from_value(serde_json::to_value(stored)?)?;
    for choice in response.choices {
        let message = rebuilt(choice.message.unwrap_or_default());
        let attrs: IndexMap<String, Value> =
            serde_json::from_value(serde_json::to_value(&message)?)?;
        history.push(Value::Instance {
            object: Instance {
                get: Getter::Mapping(attrs.clone()),
                attrs,
            },
        });
    }
    Ok(())
}

fn replayed_input(request: &IndexMap<String, Json>) -> Option<Json> {
    let input = match request.get("input") {
        Some(Json::Text(text)) if !text.is_empty() => Some(Json::Text(text.clone())),
        Some(Json::List(items)) if !items.is_empty() => Some(Json::List(items.clone())),
        Some(Json::Object(item)) => Some(Json::List(vec![Json::Object(item.clone())])),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    };
    // LiteLLM reads a stored messages value only when Python would call it truthy
    input.or_else(|| {
        request
            .get("messages")
            .filter(|messages| has_content(messages))
            .cloned()
    })
}

// redaction stores the bare sentinel, which is not JSON, so a replayed call gets "{}" a provider can parse
fn rebuilt(mut message: StoredMessage) -> StoredMessage {
    message.role = Some(message.role.unwrap_or_default());
    message.tool_calls = message.tool_calls.filter(|calls| !calls.is_empty());
    message.provider_specific_fields = message
        .provider_specific_fields
        .filter(|fields| !fields.is_empty());
    message
        .extra
        .retain(|key, value| *value != Json::Null || !DROPPED_WHEN_NULL.contains(&key.as_str()));
    let mut functions = Vec::new();
    for call in message.tool_calls.iter_mut().flatten() {
        if call.kind.is_none() {
            call.kind = Some("function".to_owned());
        }
        functions.extend(call.function.as_mut());
    }
    functions.extend(message.function_call.as_mut());
    // Function's constructor reads missing or null arguments as an empty string
    for function in functions {
        let arguments = function.arguments.get_or_insert_default();
        if arguments.as_str() == REDACTED_BY_LITELLM {
            REDACTED_ARGUMENTS.clone_into(arguments);
        }
    }
    message
}
