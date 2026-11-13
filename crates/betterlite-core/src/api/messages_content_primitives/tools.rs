use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_anthropic::feature_flags::tools::eager_input_streaming_flag;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_with::skip_serializing_none;
use sha2::{Digest as _, Sha256};

const MAX_TOOL_NAME_CHARACTERS: usize = 64;
const TOOL_NAME_PREFIX_CHARACTERS: usize = 55;
const TOOL_NAME_HASH_CHARACTERS: usize = 8;
const ANTHROPIC_TOOL_SCHEMA_KEYS: [&str; 6] = [
    "name",
    "type",
    "input_schema",
    "description",
    "cache_control",
    "strict",
];

// The Anthropic request keys the adapter turns into chat completion arguments.
pub const TRANSLATABLE_ANTHROPIC_PARAMS: [&str; 9] = [
    "messages",
    "metadata",
    "system",
    "tool_choice",
    "tools",
    "thinking",
    "output_format",
    "output_config",
    "stop_sequences",
];

#[skip_serializing_none]
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ChatToolParam {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: Object,
    pub eager_input_streaming: Option<bool>,
}

#[must_use]
pub fn truncate_tool_name(name: &str) -> String {
    if name.chars().count() <= MAX_TOOL_NAME_CHARACTERS {
        return name.to_owned();
    }
    // The hash covers the whole UTF-8 name so two names sharing a prefix stay apart.
    let digest = hex::encode(Sha256::digest(name.as_bytes()));
    let hash: String = digest.chars().take(TOOL_NAME_HASH_CHARACTERS).collect();
    let prefix: String = name.chars().take(TOOL_NAME_PREFIX_CHARACTERS).collect();
    format!("{prefix}_{hash}")
}

#[must_use]
pub fn is_provider_native_tool_dict(tool: &Object) -> bool {
    tool.len() == 1
        && tool.iter().any(|(key, value)| {
            !ANTHROPIC_TOOL_SCHEMA_KEYS.contains(&key.as_str()) && matches!(value, Json::Object(_))
        })
}

#[must_use]
pub fn is_openai_function_tool(tool: &Object) -> bool {
    match tool.get("type").and_then(Json::as_str) {
        Some("function") => tool.contains_key("function"),
        Some(_) | None => false,
    }
}

#[must_use]
pub fn is_web_search_tool(tool: &Object) -> bool {
    let typed = tool
        .get("type")
        .and_then(Json::as_str)
        .is_some_and(|kind| kind.starts_with("web_search"));
    let legacy_name = tool.get("name").and_then(Json::as_str) == Some("web_search")
        && !tool.contains_key("input_schema");
    typed || legacy_name
}

pub fn chat_tool_param(function: Object, tool: &Json) -> Result<ChatToolParam, FeatureFlagsError> {
    let eager_input_streaming = eager_input_streaming_flag(tool)?;
    Ok(ChatToolParam {
        kind: "function",
        function,
        eager_input_streaming,
    })
}
