use betterlite_models::decisions::Json;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat::response::{AssistantMessage, ChatResponse};
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use crate::message_utilities::validators::{ValidationError, convert_list_message_to_dict};

use super::responses_mcp_results::McpToolResult;

#[derive(Debug, Error)]
pub enum ChatFollowupError {
    #[error(transparent)]
    InvalidMessages(#[from] ValidationError),
    #[error("the assistant message could not be written as JSON")]
    AssistantJson(#[source] serde_json::Error),
}

// ast-grep-ignore: no-long-names
pub fn create_follow_up_messages_for_chat(
    original_messages: &[Json],
    response: &ChatResponse,
    tool_results: &[McpToolResult],
) -> Result<Vec<Json>, ChatFollowupError> {
    let mut messages = convert_list_message_to_dict(original_messages)?;
    if let Some(assistant) = assistant_message(response)? {
        messages.push(assistant);
    }
    messages.extend(tool_results.iter().map(tool_message));
    Ok(messages)
}

fn assistant_message(response: &ChatResponse) -> Result<Option<Json>, ChatFollowupError> {
    let Some(choice) = response.choices.first() else {
        tracing::error!("Failed to convert assistant message for MCP flow");
        return Ok(None);
    };
    assistant_dump(&choice.message).map(Some)
}

fn assistant_dump(message: &AssistantMessage) -> Result<Json, ChatFollowupError> {
    let mut fields = IndexMap::new();
    fields.insert(String::from("role"), Json::Text(message.role.clone()));
    if let Some(content) = &message.content {
        fields.insert(String::from("content"), Json::Text(content.clone()));
    }
    if let Some(calls) = &message.tool_calls
        && !calls.is_empty()
    {
        fields.insert(
            String::from("tool_calls"),
            Json::List(calls.iter().map(tool_call).collect()),
        );
    }
    if let Some(annotations) = &message.annotations {
        fields.insert(String::from("annotations"), annotations_json(annotations)?);
    }
    if let Some(audio) = &message.audio {
        let dumped = embedded_json(audio)?;
        fields.insert(String::from("audio"), without_null_fields(&dumped));
    }
    if let Some(refusal) = &message.refusal {
        fields.insert(String::from("refusal"), Json::Text(refusal.clone()));
    }
    Ok(Json::Object(fields))
}

fn tool_call(call: &ToolCall) -> Json {
    let mut function = IndexMap::new();
    function.insert(String::from("name"), Json::Text(call.function.name.clone()));
    function.insert(
        String::from("arguments"),
        Json::Text(call.function.arguments.clone()),
    );
    let mut fields = IndexMap::new();
    fields.insert(String::from("id"), Json::Text(call.id.clone()));
    fields.insert(String::from("type"), Json::Text(call.kind.clone()));
    fields.insert(String::from("function"), Json::Object(function));
    Json::Object(fields)
}

fn tool_message(result: &McpToolResult) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("role"), Json::Text(String::from("tool")));
    fields.insert(
        String::from("tool_call_id"),
        optional_text(result.tool_call_id.as_deref()),
    );
    fields.insert(String::from("name"), optional_text(result.name.as_deref()));
    fields.insert(String::from("content"), Json::Text(result.result.clone()));
    Json::Object(fields)
}

fn optional_text(value: Option<&str>) -> Json {
    value.map_or(Json::Null, |text| Json::Text(text.to_owned()))
}

fn annotations_json<T>(annotations: &[T]) -> Result<Json, ChatFollowupError>
where
    T: Serialize,
{
    let mut items = Vec::with_capacity(annotations.len());
    for annotation in annotations {
        let dumped = embedded_json(annotation)?;
        items.push(without_null_fields(&dumped));
    }
    Ok(Json::List(items))
}

// exclude_none drops the null fields of each dumped annotation and of the audio object, and only at their own level
fn without_null_fields(value: &Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .filter(|(_key, field)| !matches!(field, Json::Null))
                .map(|(key, field)| (key.clone(), field.clone()))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            value.clone()
        }
    }
}

fn embedded_json<T>(value: &T) -> Result<Json, ChatFollowupError>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value::<Json>)
        .map_err(ChatFollowupError::AssistantJson)
}

mod tests;
