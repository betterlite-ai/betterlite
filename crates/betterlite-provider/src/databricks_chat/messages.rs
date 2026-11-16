use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message, strip_name_from_message};
use betterlite_prompts::message_fields::image_hoist::hoist_tool_message_images;
use betterlite_prompts::message_fields::internal_fields::strip_internal_message_fields;
use betterlite_prompts::message_fields::system_runs::merge_adjacent_system_messages;
use betterlite_prompts::message_fields::tool_reference::drop_tool_reference_parts;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::request::CLAUDE;
use crate::databricks_chat::wire;

mod tests;

pub const TOOL_CALLS: &str = "tool_calls";
pub const TOOL_CALL_ID: &str = "tool_call_id";
// the parent transform's sentinel provider and a non-openai base keep Databricks's cache_control
const CONTENT_PROVIDER: &str = "openai";
const CONTENT_BASE: &str = "https://databricks.example";

// DatabricksConfig._transform_messages, ending with the parent OpenAI content item pass
pub fn transform_messages(
    messages: &[Message],
    model: &str,
    downloads: &dyn UrlDownloader,
) -> Result<Vec<Message>, DatabricksChatError> {
    let allowed_names = [String::from("user")];
    let mut kept = Vec::with_capacity(messages.len());
    for message in messages {
        let stripped = strip_name_from_message(message, &allowed_names);
        let internal = strip_internal_message_fields(&stripped);
        let moved = move_cache_control(internal);
        let sanitized = sanitize_empty_content(moved)?;
        if is_bare_assistant(&sanitized) {
            continue;
        }
        kept.push(sanitized);
    }
    let sequenced = if model.contains(CLAUDE) {
        kept
    } else {
        split_parallel_tool_calls(&merge_adjacent_system_messages(&kept))?
    };
    let stripped = drop_tool_reference_parts(&sequenced);
    rewrite_user_contents(hoist_tool_message_images(&stripped), downloads)
}

// The parent OpenAI _transform_messages rewrites the content items of user messages
fn rewrite_user_contents(
    messages: Vec<Message>,
    downloads: &dyn UrlDownloader,
) -> Result<Vec<Message>, DatabricksChatError> {
    let mut rewritten = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != "user" {
            rewritten.push(message);
            continue;
        }
        let typed: ChatMessage = wire::convert(&message)?;
        let optional_params = Object::new();
        let environment: BTreeMap<String, String> = BTreeMap::new();
        let transformed = ChatContent {
            model: "",
            messages: std::slice::from_ref(&typed),
            optional_params: &optional_params,
            provider: Some(CONTENT_PROVIDER),
            api_base: Some(CONTENT_BASE),
            global_api_base: None,
            openai_system_messages_first: false,
            environment: &environment,
            downloads,
        }
        .transform(Mode::Sync)
        .map_err(content_error)?;
        let Some(message) = transformed.messages.first() else {
            continue;
        };
        rewritten.push(wire::convert(message)?);
    }
    Ok(rewritten)
}

fn content_error(error: ChatContentError) -> DatabricksChatError {
    match error {
        ChatContentError::MissingFile(_) => DatabricksChatError::MissingFile,
        ChatContentError::Fetch(error) => DatabricksChatError::Fetch(error),
        ChatContentError::MessageShape { source } => DatabricksChatError::Decode(source),
    }
}

// _move_cache_control_into_string_content_block: only a string content moves, and only when the key is set
fn move_cache_control(mut message: Message) -> Message {
    let Some(Content::Text(text)) = &message.content else {
        return message;
    };
    let text = text.clone();
    let Some(cache_control) = message
        .provider_specific_fields
        .shift_remove("cache_control")
    else {
        return message;
    };
    let mut fields = Object::new();
    fields.insert("text".to_owned(), Json::Text(text));
    fields.insert("cache_control".to_owned(), cache_control);
    message.content = Some(Content::Parts(vec![ContentPart {
        kind: String::from("text"),
        fields,
    }]));
    message
}

// _sanitize_empty_content drops blank string content and blank text blocks
fn sanitize_empty_content(mut message: Message) -> Result<Message, DatabricksChatError> {
    let Some(content) = message.content.take() else {
        return Ok(message);
    };
    match content {
        Content::Text(text) => {
            if !text.trim().is_empty() {
                message.content = Some(Content::Text(text));
            }
            Ok(message)
        }
        Content::Parts(parts) => {
            let mut kept = Vec::with_capacity(parts.len());
            for part in parts {
                if part.kind == "text" && !text_block_has_text(&part)? {
                    continue;
                }
                kept.push(part);
            }
            if !kept.is_empty() {
                message.content = Some(Content::Parts(kept));
            }
            Ok(message)
        }
    }
}

// Python reads (block.get("text") or "").strip(), so an absent or null text counts as empty
fn text_block_has_text(part: &ContentPart) -> Result<bool, DatabricksChatError> {
    match part.fields.get("text") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Text(text)) => Ok(!text.trim().is_empty()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(DatabricksChatError::Shape {
                message: String::from("the text of a text content block must be a string"),
            })
        }
    }
}

// _is_bare_assistant_message runs after the empty content cleanup
fn is_bare_assistant(message: &Message) -> bool {
    if message.role != "assistant" {
        return false;
    }
    let has_content = message.content.is_some();
    let has_tool_calls = message
        .provider_specific_fields
        .get(TOOL_CALLS)
        .is_some_and(Json::is_truthy);
    let has_function_call = message
        .provider_specific_fields
        .get("function_call")
        .is_some_and(Json::is_truthy);
    !(has_content || has_tool_calls || has_function_call)
}

// _split_parallel_tool_calls re-emits each result right after an assistant message holding its call
fn split_parallel_tool_calls(messages: &[Message]) -> Result<Vec<Message>, DatabricksChatError> {
    let mut result = Vec::with_capacity(messages.len());
    let mut index = 0_usize;
    while let Some(message) = messages.get(index) {
        let Some(calls) = assistant_calls(message)? else {
            result.push(message.clone());
            index = index.saturating_add(1);
            continue;
        };
        let tool_messages: Vec<&Message> = messages
            .iter()
            .skip(index.saturating_add(1))
            .take_while(|next| next.role == "tool")
            .collect();
        let Some(matched) = matched_calls(calls, &tool_messages) else {
            result.push(message.clone());
            index = index.saturating_add(1);
            continue;
        };
        expand_group(message, &matched, &tool_messages, &mut result);
        index = index.saturating_add(1).saturating_add(tool_messages.len());
    }
    Ok(result)
}

// The first expanded message keeps the assistant fields, later ones carry only role and tool_calls
fn expand_group(
    message: &Message,
    matched: &[&Json],
    tool_messages: &[&Message],
    result: &mut Vec<Message>,
) {
    for (position, (call, tool_message)) in matched.iter().zip(tool_messages).enumerate() {
        let call = Json::clone(call);
        if position == 0 {
            let mut first = message.clone();
            first
                .provider_specific_fields
                .insert(TOOL_CALLS.to_owned(), Json::List(vec![call]));
            result.push(first);
        } else {
            result.push(later_call_assistant(call));
        }
        result.push(Message::clone(tool_message));
    }
}

// Only an assistant message with two or more tool calls starts a run Python would expand
fn assistant_calls(message: &Message) -> Result<Option<&[Json]>, DatabricksChatError> {
    if message.role != "assistant" {
        return Ok(None);
    }
    let Some(value) = message.provider_specific_fields.get(TOOL_CALLS) else {
        return Ok(None);
    };
    let Json::List(calls) = value else {
        if value.is_truthy() {
            return Err(DatabricksChatError::Shape {
                message: String::from("the tool_calls of an assistant message must be a list"),
            });
        }
        return Ok(None);
    };
    if calls.len() < 2 {
        return Ok(None);
    }
    Ok(Some(calls))
}

// A run expands only when every tool message answers a call and the two id sets match
fn matched_calls<'calls>(
    calls: &'calls [Json],
    tool_messages: &[&Message],
) -> Option<Vec<&'calls Json>> {
    let mut by_id: BTreeMap<&'calls str, &'calls Json> = BTreeMap::new();
    for call in calls {
        let id = call.as_object()?.get("id")?.as_str()?;
        by_id.insert(id, call);
    }
    let mut matched = Vec::with_capacity(tool_messages.len());
    let mut result_ids: BTreeSet<&str> = BTreeSet::new();
    for message in tool_messages {
        let id = message
            .provider_specific_fields
            .get(TOOL_CALL_ID)?
            .as_str()?;
        result_ids.insert(id);
        matched.push(*by_id.get(id)?);
    }
    let call_ids: BTreeSet<&str> = by_id.keys().copied().collect();
    if matched.len() != calls.len() || call_ids != result_ids {
        return None;
    }
    Some(matched)
}

// Later expanded messages carry exactly role and tool_calls
fn later_call_assistant(call: Json) -> Message {
    let mut fields = Object::new();
    fields.insert(TOOL_CALLS.to_owned(), Json::List(vec![call]));
    Message {
        role: String::from("assistant"),
        content: None,
        name: None,
        provider_specific_fields: fields,
    }
}
