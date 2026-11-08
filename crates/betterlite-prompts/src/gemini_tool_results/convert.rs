use crate::arguments::json::{Json, Object};
use crate::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use crate::gemini_tool_invokes::signatures::THOUGHT_SIGNATURE_SEPARATOR;
use crate::gemini_tool_results::content::collect_content;
use crate::gemini_tool_results::error::GeminiToolResultsError;
use crate::gemini_tool_results::types::{
    GeminiFunctionResponse, GeminiFunctionResponsePart, GeminiResponseMedia, GeminiToolResult,
    LastToolCallMessage,
};
use crate::media::inline::RemoteMediaFetcher;
use crate::message_content::Message;

const TOOL_ROLE: &str = "tool";
const FUNCTION_ROLE: &str = "function";
const TOOL_CALL_ID: &str = "tool_call_id";
const CONTENT: &str = "content";

pub async fn convert_tool_result_to_gemini<Source>(
    message: &Message,
    last_message_with_tool_calls: Option<&LastToolCallMessage>,
    forward_function_call_id: bool,
    fetcher: &Source,
) -> Result<GeminiToolResult, GeminiToolResultsError>
where
    Source: RemoteMediaFetcher + Sync,
{
    if message.role != TOOL_ROLE && message.role != FUNCTION_ROLE {
        return Err(GeminiToolResultsError::Role {
            role: message.role.clone(),
        });
    }
    let collected = collect_content(message.content.as_ref(), fetcher).await?;
    let name = resolve_name(message, last_message_with_tool_calls)?;
    let id = forwarded_call_id(message, forward_function_call_id)?;
    if name.is_empty() {
        return Err(GeminiToolResultsError::MissingName);
    }
    let media = collected
        .media
        .into_iter()
        .map(|inline_data| GeminiResponseMedia { inline_data })
        .collect::<Vec<_>>();
    let part = GeminiFunctionResponsePart {
        function_response: GeminiFunctionResponse {
            name,
            response: response_data(&collected.text),
            id,
            parts: media,
        },
    };
    if part.function_response.parts.is_empty() {
        return Ok(GeminiToolResult::Single(part));
    }
    Ok(GeminiToolResult::WithMedia(vec![part]))
}

fn resolve_name(
    message: &Message,
    last_message_with_tool_calls: Option<&LastToolCallMessage>,
) -> Result<String, GeminiToolResultsError> {
    let mut name = message.name.clone().unwrap_or_default();
    let Some(last_message) = last_message_with_tool_calls else {
        return Ok(name);
    };
    let tools = last_message.tool_calls.as_deref().unwrap_or_default();
    if tools.len() > MAX_TOOL_CALLS {
        return Err(GeminiToolResultsError::TooManyToolCalls {
            limit: MAX_TOOL_CALLS,
        });
    }
    let Some(message_id) = optional_string(message, TOOL_CALL_ID)? else {
        return Ok(name);
    };
    if message_id.is_empty() {
        return Ok(name);
    }
    for tool in tools {
        let Some(previous_id) = tool.id.as_deref() else {
            continue;
        };
        if previous_id.is_empty() || previous_id != message_id {
            continue;
        }
        name = tool
            .function
            .as_ref()
            .and_then(|function| function.name.clone())
            .unwrap_or_default();
    }
    Ok(name)
}

fn forwarded_call_id(
    message: &Message,
    forward_function_call_id: bool,
) -> Result<Option<String>, GeminiToolResultsError> {
    if !forward_function_call_id {
        return Ok(None);
    }
    let Some(raw) = optional_string(message, TOOL_CALL_ID)? else {
        return Ok(None);
    };
    let clean = raw
        .split_once(THOUGHT_SIGNATURE_SEPARATOR)
        .map_or(raw.as_str(), |(head, _signature)| head);
    if clean.is_empty() {
        return Ok(None);
    }
    Ok(Some(clean.to_owned()))
}

fn optional_string(
    message: &Message,
    field: &str,
) -> Result<Option<String>, GeminiToolResultsError> {
    match message.provider_specific_fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(GeminiToolResultsError::Field {
            field: field.to_owned(),
            expected: "a string",
        }),
    }
}

fn response_data(content: &str) -> Json {
    if content.trim_start().starts_with(['{', '['])
        && let Ok(Json::Object(object)) = serde_json::from_str::<Json>(content)
    {
        return Json::Object(object);
    }
    Json::Object(Object::from_iter([(
        CONTENT.to_owned(),
        Json::Text(content.to_owned()),
    )]))
}
