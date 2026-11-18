use betterlite_openai::base_helpers::DUMMY_TOOLS;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{
    Content, ContentPart, Message, convert_content_list_to_str,
};

use crate::converse_messages::access::{
    assistant_has_tool_calls, has_orphaned_tool_blocks, tool_calls,
};
use crate::converse_messages::error::ConverseMessageError;
use crate::converse_params::wire::{carries_value, transcode};
use crate::model_routes::ParamMap;

const ARGUMENTS_FIELD: &str = "arguments";
const FUNCTION_FIELD: &str = "function";
const GUARDRAIL_FIELD: &str = "guardrailConfig";
const GUARDED_TEXT: &str = "guarded_text";
const ID_FIELD: &str = "id";
const NAME_FIELD: &str = "name";
const TEXT_FIELD: &str = "text";
const TOOL_CALL_ID_FIELD: &str = "tool_call_id";
const TOOLS_FIELD: &str = "tools";
const UNKNOWN_RESULT: &str = "unknown";
const UNKNOWN_TOOL: &str = "unknown_tool";
const OMITTED_RESULT: &str = "<non-text tool result omitted>";
const NEUTRALIZED_WARNING: &str = "litellm.bedrock: request has tool blocks in message history but no \
`tools=` param; neutralizing orphaned tool blocks to text so Bedrock accepts the request without a \
toolConfig. Non-text tool-result payloads are dropped. Pass `tools=` to preserve structured tool calling.";

// The two switches the orphan policy reads; LiteLLM keeps them on litellm as globals.
#[derive(Clone, Copy, Debug)]
pub struct OrphanPolicy {
    pub neutralize: bool,
    pub modify_params: bool,
}

// The default policy: rewrite orphaned tool blocks to text so Bedrock accepts the request without a toolConfig.
pub fn handle_orphaned_tool_blocks(
    messages: &[Message],
    optional_params: &mut ParamMap,
    policy: OrphanPolicy,
) -> Result<Vec<Message>, ConverseMessageError> {
    if policy.neutralize {
        return Ok(neutralize_orphaned_blocks(messages, optional_params));
    }
    // Any role may carry the blocks, and LiteLLM counts an empty tool_calls list as a block.
    let call_blocks = messages.iter().any(|message| tool_calls(message).is_some());
    if optional_params.contains_key(TOOLS_FIELD) || !call_blocks {
        return Ok(messages.to_vec());
    }
    if policy.modify_params {
        optional_params.insert(TOOLS_FIELD.to_owned(), transcode(&DUMMY_TOOLS)?);
        return Ok(messages.to_vec());
    }
    Err(ConverseMessageError::ToolCallsWithoutTools)
}

#[must_use]
pub fn neutralize_orphaned_blocks(
    messages: &[Message],
    optional_params: &ParamMap,
) -> Vec<Message> {
    let tools_present = optional_params.get(TOOLS_FIELD).is_some_and(carries_value);
    if tools_present || !has_orphaned_tool_blocks(messages) {
        return messages.to_vec();
    }
    tracing::warn!("{NEUTRALIZED_WARNING}");
    let guardrail_active = optional_params.contains_key(GUARDRAIL_FIELD);
    messages
        .iter()
        .map(|message| rewrite(message, guardrail_active))
        .collect()
}

fn rewrite(message: &Message, guardrail_active: bool) -> Message {
    if assistant_has_tool_calls(message) {
        return assistant_projection(message);
    }
    if matches!(message.role.as_str(), "tool" | "function") {
        return result_projection(message, guardrail_active);
    }
    message.clone()
}

// The assistant keeps its flattened text and gains one line per call; everything else on it is dropped.
fn assistant_projection(message: &Message) -> Message {
    let base_text = convert_content_list_to_str(message);
    let mut parts = Vec::new();
    if !base_text.is_empty() {
        parts.push(base_text);
    }
    if let Some(Json::List(calls)) = tool_calls(message) {
        for call in calls {
            parts.push(tool_call_text(call));
        }
    }
    Message {
        role: "assistant".to_owned(),
        content: Some(Content::Text(parts.join("\n"))),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

// Python's str.strip() also counts the four C0 separators as whitespace, which Rust's trim() does not.
fn stripped(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

fn tool_call_text(call: &Json) -> String {
    let fields = call.as_object();
    let call_id = fields
        .and_then(|call_fields| call_fields.get(ID_FIELD))
        .and_then(Json::as_str)
        .filter(|call_id| !call_id.is_empty());
    let function = fields
        .and_then(|call_fields| call_fields.get(FUNCTION_FIELD))
        .and_then(Json::as_object);
    let name = function
        .and_then(|function_fields| function_fields.get(NAME_FIELD))
        .and_then(Json::as_str)
        .filter(|name| !name.is_empty());
    let arguments = function
        .and_then(|function_fields| function_fields.get(ARGUMENTS_FIELD))
        .and_then(Json::as_str)
        .filter(|arguments| !arguments.is_empty());
    let label = call_id.map_or_else(|| "tool call".to_owned(), |id| format!("tool call {id}"));
    format!(
        "[{label}: {}({})]",
        name.unwrap_or(UNKNOWN_TOOL),
        arguments.unwrap_or_default()
    )
}

// A tool result is external input, so a guardrail wraps it here too, wherever it sits in history.
fn result_projection(message: &Message, guardrail_active: bool) -> Message {
    let tool_call_id = message
        .provider_specific_fields
        .get(TOOL_CALL_ID_FIELD)
        .and_then(Json::as_str)
        .filter(|tool_call_id| !tool_call_id.is_empty());
    let name = message.name.as_deref().filter(|name| !name.is_empty());
    let label = tool_call_id.or(name).unwrap_or(UNKNOWN_RESULT);
    let flattened = convert_content_list_to_str(message);
    let trimmed = stripped(&flattened);
    let rendered = if trimmed.is_empty() {
        OMITTED_RESULT
    } else {
        trimmed
    };
    let result_text = format!("[tool result for {label}: {rendered}]");
    let content = if guardrail_active {
        Content::Parts(vec![ContentPart {
            kind: GUARDED_TEXT.to_owned(),
            fields: Object::from_iter([(TEXT_FIELD.to_owned(), Json::Text(result_text))]),
        }])
    } else {
        Content::Text(result_text)
    };
    Message {
        role: "user".to_owned(),
        content: Some(content),
        name: None,
        provider_specific_fields: Object::new(),
    }
}
