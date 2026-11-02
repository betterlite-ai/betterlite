use crate::arguments::json::Json;
use crate::message_content::ContentPart;

pub(crate) fn readable_thinking_text(block: &ContentPart) -> String {
    if block.kind != "thinking" {
        return String::new();
    }
    // A "thinking" field of the wrong JSON type is treated as absent, matching image_hoist.rs.
    block
        .fields
        .get("thinking")
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned()
}

// Redacted blocks carry no readable text, so readable_thinking_text contributes nothing for them.
#[must_use]
pub fn reasoning_content_from_blocks(thinking_blocks: &[ContentPart]) -> String {
    thinking_blocks
        .iter()
        .map(readable_thinking_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

mod tests;
