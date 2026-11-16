use betterlite_models::fireworks_catalog_metadata::{Capability, Catalog, capability_exact};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_fields::nested_filter::filter_value_from_dict;
use serde_json::Value;

use crate::chat::ChatMessage;
use crate::fireworks_ai_chat::error::FireworksChatError;

const ROLE: &str = "role";
const CONTENT: &str = "content";
const TYPE: &str = "type";
const FILE: &str = "file";
const IMAGE_URL: &str = "image_url";
const USER: &str = "user";
const CACHE_CONTROL: &str = "cache_control";
const PROVIDER_SPECIFIC_FIELDS: &str = "provider_specific_fields";
const THINKING_BLOCKS: &str = "thinking_blocks";

// _transform_messages_helper: the two 400 checks first, then the keys Fireworks rejects
pub(crate) fn transform_messages(
    messages: &[ChatMessage],
    model: &str,
    catalog: &Catalog,
) -> Result<Vec<ChatMessage>, FireworksChatError> {
    let vision = capability_exact(catalog, model, Capability::Vision);
    let mut transformed = Vec::with_capacity(messages.len());
    for message in messages {
        transformed.push(cleaned_message(message, model, vision)?);
    }
    Ok(transformed)
}

fn cleaned_message(
    message: &ChatMessage,
    model: &str,
    vision: Option<bool>,
) -> Result<ChatMessage, FireworksChatError> {
    let raw = serde_json::to_value(message)
        .map_err(|source| FireworksChatError::MessageShape { source })?;
    scan_user_content(&raw, model, vision)?;
    let fields: Object = serde_json::from_value(raw)
        .map_err(|source| FireworksChatError::MessageShape { source })?;
    let mut filtered = filter_value_from_dict(&fields, CACHE_CONTROL, 0);
    filtered.shift_remove(PROVIDER_SPECIFIC_FIELDS);
    filtered.shift_remove(THINKING_BLOCKS);
    let value = serde_json::to_value(filtered)
        .map_err(|source| FireworksChatError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| FireworksChatError::MessageShape { source })
}

// a user content list is scanned in order, and anything that is not a map is skipped
fn scan_user_content(
    message: &Value,
    model: &str,
    vision: Option<bool>,
) -> Result<(), FireworksChatError> {
    if message.get(ROLE).and_then(Value::as_str) != Some(USER) {
        return Ok(());
    }
    let Some(Value::Array(blocks)) = message.get(CONTENT) else {
        return Ok(());
    };
    for block in blocks {
        let Some(fields) = block.as_object() else {
            continue;
        };
        match fields.get(TYPE).and_then(Value::as_str) {
            Some(FILE) => return Err(FireworksChatError::FileContent),
            Some(IMAGE_URL) if vision == Some(false) => {
                return Err(FireworksChatError::ImageContent {
                    model: model.to_owned(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}
