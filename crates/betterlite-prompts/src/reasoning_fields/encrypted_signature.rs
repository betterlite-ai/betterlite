use crate::arguments::json::Json;
use crate::message_content::ContentPart;

pub const ENCRYPTED_REASONING_PREFIX: &str = "litellm_encrypted_reasoning:";

// Anthropic echoes a thinking block's signature or a redacted block's data back verbatim, so either field can carry this prefix.
#[must_use]
pub fn encrypted_reasoning_signature(encrypted_content: &str) -> String {
    let mut signature = ENCRYPTED_REASONING_PREFIX.to_owned();
    signature.push_str(encrypted_content);
    signature
}

#[must_use]
pub fn encrypted_content_of_signature(signature: &Json) -> Option<String> {
    let Json::Text(text) = signature else {
        return None;
    };
    let suffix = text.strip_prefix(ENCRYPTED_REASONING_PREFIX)?;
    (!suffix.is_empty()).then(|| suffix.to_owned())
}

fn encrypted_reasoning_field(block: &ContentPart) -> Json {
    match block.kind.as_str() {
        "thinking" => block.fields.get("signature").cloned().unwrap_or(Json::Null),
        "redacted_thinking" => block.fields.get("data").cloned().unwrap_or(Json::Null),
        _ => Json::Null,
    }
}

#[must_use]
pub fn encrypted_content_of_block(block: &ContentPart) -> Option<String> {
    match encrypted_reasoning_field(block) {
        Json::Text(value) => encrypted_content_of_signature(&Json::Text(value)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// Only the Responses API that minted the content can read it back, so an Anthropic backend drops the block rather than failing to verify it.
#[must_use]
pub fn is_encrypted_reasoning_block(block: &ContentPart) -> bool {
    match encrypted_reasoning_field(block) {
        Json::Text(value) => value.starts_with(ENCRYPTED_REASONING_PREFIX),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    }
}

// Anthropic rejects a null, empty or foreign-provider signature, and a thinking block whose text is empty or whitespace-only; redacted_thinking carries no signature and is always kept.
#[must_use]
pub fn is_unsignable_thinking_block(block: &ContentPart) -> bool {
    if is_encrypted_reasoning_block(block) {
        return true;
    }
    if block.kind != "thinking" {
        return false;
    }
    let has_signature =
        matches!(block.fields.get("signature"), Some(Json::Text(value)) if !value.is_empty());
    if !has_signature {
        return true;
    }
    let has_thinking_text =
        matches!(block.fields.get("thinking"), Some(Json::Text(value)) if !value.trim().is_empty());
    !has_thinking_text
}

mod tests;
