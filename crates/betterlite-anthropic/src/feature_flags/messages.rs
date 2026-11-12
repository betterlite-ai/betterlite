use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::file_ids::messages::get_file_ids_from_messages;
use betterlite_prompts::message_content::Message;

use super::FeatureFlagsError;

#[must_use]
pub fn is_cache_control_set(messages: &[Json]) -> bool {
    messages.iter().any(|message| {
        let Some(fields) = message.as_object() else {
            return false;
        };
        let present = fields
            .get("cache_control")
            .is_some_and(|value| !matches!(value, Json::Null));
        if present {
            return true;
        }
        let Some(Json::List(blocks)) = fields.get("content") else {
            return false;
        };
        blocks.iter().any(|block| {
            block
                .as_object()
                .is_some_and(|block_fields| block_fields.contains_key("cache_control"))
        })
    })
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
    messages.iter().any(|message| {
        let Some(fields) = message.as_object() else {
            return false;
        };
        if fields.get("role").and_then(Json::as_str) != Some("system") {
            return false;
        }
        let Some(Json::List(blocks)) = fields.get("content") else {
            return false;
        };
        blocks.iter().any(is_tool_change_block)
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
pub const fn is_mcp_server_used(mcp_servers: Option<&[Json]>) -> bool {
    let Some(servers) = mcp_servers else {
        return false;
    };
    !servers.is_empty()
}

#[must_use]
pub fn is_pdf_used(messages: &[Json]) -> bool {
    messages.iter().any(|message| {
        let Some(fields) = message.as_object() else {
            return false;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            return false;
        };
        blocks.iter().any(is_non_text_block)
    })
}

fn is_non_text_block(block: &Json) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    match fields.get("type") {
        Some(Json::Text(text)) => text != "text",
        Some(_) => true,
        None => false,
    }
}
