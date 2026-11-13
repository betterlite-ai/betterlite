use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

// A tool call that carries attributes; LiteLLM reads those with getattr, so a plain mapping has none.
#[derive(Debug, Default, Deserialize)]
pub struct ToolCallFields {
    #[serde(default)]
    pub provider_specific_fields: Object,
    #[serde(default)]
    pub function: FunctionFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct FunctionFields {
    #[serde(default)]
    pub provider_specific_fields: Object,
}

// A non-empty outer mapping decides alone, even when it has no thought_signature.
#[must_use]
pub fn tool_call_signature(tool_call: &ToolCallFields) -> Option<&str> {
    let outer = &tool_call.provider_specific_fields;
    let fields = if outer.is_empty() {
        &tool_call.function.provider_specific_fields
    } else {
        outer
    };
    fields.get("thought_signature").and_then(Json::as_str)
}

#[must_use]
pub fn tool_use_signature(content: &Object) -> Option<&str> {
    content
        .get("provider_specific_fields")
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("signature"))
        .and_then(Json::as_str)
}

#[must_use]
pub fn compaction_blocks(fields: &Object) -> Vec<&Object> {
    let Some(Json::List(blocks)) = fields.get("compaction_blocks") else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(Json::as_object)
        .filter(|block| block.get("type").and_then(Json::as_str) == Some("compaction"))
        .collect()
}
