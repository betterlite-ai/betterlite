use crate::arguments::json::Json;

#[must_use]
pub fn drop_unsignable(blocks: &[Json]) -> Vec<Json> {
    blocks
        .iter()
        .filter(|block| !is_unsignable_thinking_block(block))
        .cloned()
        .collect()
}

fn is_unsignable_thinking_block(block: &Json) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    let kind = fields.get("type").and_then(Json::as_str);
    let encrypted_value = match kind {
        Some("thinking") => fields.get("signature"),
        Some("redacted_thinking") => fields.get("data"),
        _ => None,
    };
    if has_encrypted_marker(encrypted_value) {
        return true;
    }
    if kind != Some("thinking") {
        return false;
    }
    let signature_value = fields.get("signature").and_then(Json::as_str);
    let thinking_value = fields.get("thinking").and_then(Json::as_str);
    signature_value.is_none_or(str::is_empty)
        || thinking_value.is_none_or(|thinking_text| thinking_text.trim().is_empty())
}

fn has_encrypted_marker(value: Option<&Json>) -> bool {
    value
        .and_then(Json::as_str)
        .is_some_and(|signature| signature.starts_with("litellm_encrypted_reasoning:"))
}
