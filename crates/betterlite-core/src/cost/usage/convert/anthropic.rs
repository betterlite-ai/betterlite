use betterlite_anthropic::usage::{
    AnthropicUsage, CacheCreationTokenDetails,
    CompletionTokensDetails as AnthropicCompletionDetails,
    PromptTokensDetails as AnthropicPromptDetails, ServerToolUse,
};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::insert_count;
pub(super) fn from_usage(usage: &AnthropicUsage) -> Usage {
    Usage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: Some(prompt_details(&usage.prompt_tokens_details)),
        completion_tokens_details: Some(completion_details(&usage.completion_tokens_details)),
        server_tool_use: usage.server_tool_use.as_ref().map(server_tool_use),
        cost: None,
        extras: extras(usage),
    }
}
fn prompt_details(details: &AnthropicPromptDetails) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    for (name, value) in [
        ("text_tokens", Some(details.text)),
        ("cached_tokens", Some(details.cached)),
        ("cache_creation_tokens", Some(details.cache_creation)),
        ("cache_write_tokens", Some(details.cache_write)),
        ("audio_tokens", details.audio),
        ("image_tokens", details.image),
        ("video_tokens", details.video),
    ] {
        insert_count(&mut record, name, value);
    }
    if let Some(nested) = details.cache_creation_details.as_ref() {
        record.insert(
            "cache_creation_token_details".to_owned(),
            cache_creation_details(nested),
        );
    }
    record
}
fn cache_creation_details(details: &CacheCreationTokenDetails) -> Json {
    let mut record = IndexMap::new();
    insert_count(
        &mut record,
        "ephemeral_5m_input_tokens",
        details.ephemeral_5m,
    );
    insert_count(
        &mut record,
        "ephemeral_1h_input_tokens",
        details.ephemeral_1h,
    );
    Json::Object(record)
}
fn completion_details(details: &AnthropicCompletionDetails) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    insert_count(&mut record, "reasoning_tokens", details.reasoning);
    insert_count(&mut record, "text_tokens", details.text);
    insert_count(&mut record, "audio_tokens", details.audio);
    insert_count(&mut record, "image_tokens", details.image);
    insert_count(&mut record, "video_tokens", details.video);
    insert_count(
        &mut record,
        "accepted_prediction_tokens",
        details.accepted_prediction,
    );
    insert_count(
        &mut record,
        "rejected_prediction_tokens",
        details.rejected_prediction,
    );
    record
}
fn server_tool_use(usage: &ServerToolUse) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    insert_count(&mut record, "web_search_requests", usage.web_search);
    insert_count(&mut record, "tool_search_requests", usage.tool_search);
    insert_count(&mut record, "browser_open_requests", usage.browser_open);
    record
}
fn extras(usage: &AnthropicUsage) -> IndexMap<String, Json> {
    let mut extras = IndexMap::new();
    insert_count(
        &mut extras,
        "cache_creation_input_tokens",
        Some(usage.cache_creation_input_tokens),
    );
    insert_count(
        &mut extras,
        "cache_read_input_tokens",
        Some(usage.cache_read_input_tokens),
    );
    extras.insert(
        "iterations".to_owned(),
        usage.iterations.clone().unwrap_or(Json::Null),
    );
    insert_text(&mut extras, "inference_geo", usage.inference_geo.as_deref());
    insert_text(&mut extras, "speed", usage.speed.as_deref());
    insert_text(&mut extras, "service_tier", usage.service_tier.as_deref());
    extras
}
fn insert_text(record: &mut IndexMap<String, Json>, name: &str, value: Option<&str>) {
    record.insert(
        name.to_owned(),
        value.map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
}
