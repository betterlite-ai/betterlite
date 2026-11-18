use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, Message};

const TOOL_CALLS_FIELD: &str = "tool_calls";

// LiteLLM reads tool_calls off the message dict, and a null value counts as absent.
#[must_use]
pub(crate) fn tool_calls(message: &Message) -> Option<&Json> {
    message
        .provider_specific_fields
        .get(TOOL_CALLS_FIELD)
        .filter(|value| !matches!(value, Json::Null))
}

#[must_use]
pub(crate) fn assistant_has_tool_calls(message: &Message) -> bool {
    if message.role != "assistant" {
        return false;
    }
    tool_calls(message).is_some_and(Json::is_truthy)
}

#[must_use]
pub(crate) fn has_orphaned_tool_blocks(messages: &[Message]) -> bool {
    messages.iter().any(|message| {
        assistant_has_tool_calls(message) || matches!(message.role.as_str(), "tool" | "function")
    })
}

// B221's accessors: only the first part of a user turn opens a result run, and function turns never do.
#[must_use]
pub(crate) fn opens_with_tool_result(message: &Message) -> bool {
    match message.role.as_str() {
        "tool" => true,
        "user" => match message.content.as_ref() {
            Some(Content::Parts(parts)) => {
                parts.first().is_some_and(|part| part.kind == "tool_result")
            }
            Some(Content::Text(_)) | None => false,
        },
        _ => false,
    }
}
