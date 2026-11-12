use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::file_ids::messages::get_file_ids_from_messages;
use betterlite_prompts::message_content::Message;

use super::FeatureFlagsError;

#[must_use]
pub fn is_cache_control_set(messages: &[Json]) -> bool {
    any_message(messages, |fields| {
        if fields
            .get("cache_control")
            .is_some_and(|value| !matches!(value, Json::Null))
        {
            return true;
        }
        content_blocks(fields).is_some_and(|blocks| blocks.iter().any(block_has_cache_control))
    })
}

fn block_has_cache_control(block: &Json) -> bool {
    block
        .as_object()
        .is_some_and(|block_fields| block_fields.contains_key("cache_control"))
}

pub fn is_file_id_used(messages: &[Json]) -> Result<bool, FeatureFlagsError> {
    let typed = decode_messages(messages)?;
    let ids = get_file_ids_from_messages(&typed)?;
    Ok(!ids.is_empty())
}

fn decode_messages(messages: &[Json]) -> Result<Vec<Message>, FeatureFlagsError> {
    let mut decoded = Vec::with_capacity(messages.len());
    for message in messages {
        let value = serde_json::to_value(message).map_err(FeatureFlagsError::Messages)?;
        decoded.push(serde_json::from_value(value).map_err(FeatureFlagsError::Messages)?);
    }
    Ok(decoded)
}

#[must_use]
pub fn has_thinking_display_updates(thinking: Option<&Json>) -> bool {
    let Some(Json::Object(fields)) = thinking else {
        return false;
    };
    if !matches!(
        fields.get("type").and_then(Json::as_str),
        Some("adaptive" | "enabled")
    ) {
        return false;
    }
    fields.get("display").and_then(Json::as_str) == Some("updates")
}

#[must_use]
pub fn has_tool_change(messages: &[Json]) -> bool {
    any_message(messages, |fields| {
        fields.get("role").and_then(Json::as_str) == Some("system")
            && content_blocks(fields).is_some_and(|blocks| blocks.iter().any(is_tool_change_block))
    })
}

fn is_tool_change_block(block: &Json) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    let changed = matches!(
        fields.get("type").and_then(Json::as_str),
        Some("tool_addition" | "tool_removal")
    );
    let Some(Json::Object(tool)) = fields.get("tool") else {
        return false;
    };
    changed && tool.get("type").and_then(Json::as_str) == Some("tool_reference")
}

#[must_use]
pub fn has_output_config(messages: &[Json]) -> bool {
    messages.iter().any(|message| {
        message
            .as_object()
            .is_some_and(|fields| fields.contains_key("output_config"))
    })
}

#[must_use]
pub fn is_pdf_used(messages: &[Json]) -> bool {
    messages
        .iter()
        .filter_map(Json::as_object)
        .flat_map(|fields| content_blocks(fields).unwrap_or_default())
        .any(|block| block.as_object().is_some_and(is_non_text))
}

fn is_non_text(fields: &Object) -> bool {
    match fields.get("type") {
        Some(Json::Text(text)) => text != "text",
        Some(_) => true,
        None => false,
    }
}

fn any_message<F>(messages: &[Json], matches: F) -> bool
where
    F: Fn(&Object) -> bool,
{
    for message in messages {
        if message.as_object().is_some_and(&matches) {
            return true;
        }
    }
    false
}

fn content_blocks(fields: &Object) -> Option<&[Json]> {
    match fields.get("content") {
        Some(Json::List(blocks)) => Some(blocks),
        Some(_) | None => None,
    }
}
