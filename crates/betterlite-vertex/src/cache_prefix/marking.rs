use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, Message};

const CACHE_CONTROL: &str = "cache_control";
const TYPE: &str = "type";
const EPHEMERAL: &str = "ephemeral";
const TEXT: &str = "text";

#[must_use]
pub fn is_cached_message(message: &Message, disable_transform: bool) -> bool {
    if disable_transform {
        return false;
    }
    if is_ephemeral(message.provider_specific_fields.get(CACHE_CONTROL)) {
        return true;
    }
    let Some(Content::Parts(items)) = &message.content else {
        return false;
    };
    items
        .iter()
        .any(|item| item.kind == TEXT && is_ephemeral(item.fields.get(CACHE_CONTROL)))
}

fn is_ephemeral(cache_control: Option<&Json>) -> bool {
    cache_control
        .and_then(Json::as_object)
        .and_then(|fields| fields.get(TYPE))
        .and_then(Json::as_str)
        == Some(EPHEMERAL)
}
