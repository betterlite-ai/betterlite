use betterlite_core::api::response_id_codecs::encrypted::{
    decode_encrypted_item_id, unwrap_encrypted_content,
};
use betterlite_core::api::response_id_traversal::record::Record;
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::reasoning_fields::encrypted_signature::encrypted_content_of_block;

// _extract_model_id_from_input: only a list is scanned, in order, and the first encoded item decides.
#[must_use]
pub fn input_model_id(input: Option<&Record>) -> Option<String> {
    let Record::List(items) = input? else {
        return None;
    };
    items.iter().find_map(item_model_id)
}

// _model_id_of_input_item: the encoded item id first, then the wrapped encrypted_content.
#[must_use]
pub fn item_model_id(item: &Record) -> Option<String> {
    let item_id = item
        .field("id")
        .and_then(Record::as_text)
        .filter(|id| !id.is_empty());
    if let Some(decoded) = item_id.and_then(decode_encrypted_item_id) {
        return Some(decoded.model_id);
    }
    item.field("encrypted_content")
        .and_then(Record::as_text)
        .filter(|content| !content.is_empty())
        .and_then(wrapped_model_id)
}

// _extract_model_id_from_anthropic_messages: the first bridged thinking signature or redacted data in content order.
#[must_use]
pub fn anthropic_model_id(messages: &[Message]) -> Option<String> {
    if messages.is_empty() {
        return None;
    }
    messages.iter().find_map(message_model_id)
}

fn message_model_id(message: &Message) -> Option<String> {
    let Some(Content::Parts(blocks)) = &message.content else {
        return None;
    };
    blocks.iter().find_map(|block| {
        encrypted_content_of_block(block)
            .as_deref()
            .and_then(wrapped_model_id)
    })
}

// _model_id_from_wrapped_encrypted_content: an empty metadata id carries no marker.
#[must_use]
pub fn wrapped_model_id(content: &str) -> Option<String> {
    let (model_id, _restored) = unwrap_encrypted_content(content);
    let decoded = model_id?;
    (!decoded.is_empty()).then_some(decoded)
}
