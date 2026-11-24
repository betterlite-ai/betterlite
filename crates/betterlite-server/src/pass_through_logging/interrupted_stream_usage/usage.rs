use betterlite_logging::fields::usage::Usage as LoggingUsage;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject as ShapedObject, JsonValue as ShapedValue,
};
use betterlite_streaming::stream_usage_assembly::wire::{
    PromptTokensDetails, ServerToolUse, Usage,
};
use betterlite_streaming::streamed_response_assembly::response::Response;
use indexmap::IndexMap;
use serde_json::Number;

// B251's record_partial_usage_for_failure takes this usage record; both assembled variants carry one.
pub(super) fn usage_of(response: &Response) -> LoggingUsage {
    match response {
        Response::Chat(chat) => logging_usage(&chat.usage),
        Response::Text(text) => LoggingUsage {
            prompt_tokens: text.usage.prompt_tokens,
            completion_tokens: text.usage.completion_tokens,
            total_tokens: text.usage.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            server_tool_use: None,
            cost: None,
            extras: IndexMap::new(),
        },
    }
}

fn logging_usage(usage: &Usage) -> LoggingUsage {
    let mut extras = IndexMap::new();
    for (name, count) in [
        ("cache_read_input_tokens", usage.cache_read_input_tokens),
        (
            "cache_creation_input_tokens",
            usage.cache_creation_input_tokens,
        ),
    ] {
        if let Some(reported) = count {
            extras.insert(name.to_owned(), Json::Number(Number::from(reported)));
        }
    }
    for (name, label) in [
        ("inference_geo", usage.inference_geo.as_deref()),
        ("speed", usage.speed.as_deref()),
    ] {
        if let Some(text) = label {
            extras.insert(name.to_owned(), Json::Text(text.to_owned()));
        }
    }
    LoggingUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: usage.prompt_tokens_details.as_ref().map(prompt_details),
        completion_tokens_details: completion_details(usage),
        server_tool_use: usage.server_tool_use.as_ref().map(server_details),
        cost: usage.cost,
        extras,
    }
}

fn prompt_details(details: &PromptTokensDetails) -> IndexMap<String, Json> {
    let mut fields = detail_map(
        &[
            ("cached_tokens", details.cached_tokens),
            ("web_search_requests", details.web_search_requests),
            (
                "google_maps_grounding_requests",
                details.google_maps_grounding_requests,
            ),
            ("cache_write_tokens", details.cache_write_tokens),
            ("cache_creation_tokens", details.cache_creation_tokens),
        ],
        Some(&details.extra),
    );
    if let Some(creation) = &details.cache_creation_token_details {
        fields.insert(
            "cache_creation_token_details".to_owned(),
            Json::Object(detail_map(
                &[
                    (
                        "ephemeral_5m_input_tokens",
                        creation.ephemeral_5m_input_tokens,
                    ),
                    (
                        "ephemeral_1h_input_tokens",
                        creation.ephemeral_1h_input_tokens,
                    ),
                ],
                None,
            )),
        );
    }
    fields
}

fn completion_details(usage: &Usage) -> Option<IndexMap<String, Json>> {
    let details = usage.completion_tokens_details.as_ref()?;
    Some(detail_map(
        &[
            ("reasoning_tokens", details.reasoning_tokens),
            ("text_tokens", details.text_tokens),
            ("image_tokens", details.image_tokens),
            ("audio_tokens", details.audio_tokens),
        ],
        Some(&details.extra),
    ))
}

fn server_details(details: &ServerToolUse) -> IndexMap<String, Json> {
    detail_map(
        &[
            ("web_search_requests", details.web_search_requests),
            ("tool_search_requests", details.tool_search_requests),
            ("browser_open_requests", details.browser_open_requests),
        ],
        None,
    )
}

// The detail groups differ only in their count list and whether they carry untyped extras.
fn detail_map(
    entries: &[(&str, Option<i64>)],
    extra: Option<&ShapedObject>,
) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    for &(name, value) in entries {
        if let Some(number) = value {
            fields.insert(name.to_owned(), Json::Number(Number::from(number)));
        }
    }
    if let Some(extras) = extra {
        copy_extra(&mut fields, extras);
    }
    fields
}

// The fields the wire usage leaves untyped are copied as they arrived, so no count is dropped.
fn copy_extra(fields: &mut IndexMap<String, Json>, extra: &ShapedObject) {
    for (name, value) in extra {
        fields.insert(name.to_owned(), decisions_json(value));
    }
}

fn decisions_json(value: &ShapedValue) -> Json {
    match value {
        ShapedValue::Null => Json::Null,
        ShapedValue::Bool(flag) => Json::Bool(*flag),
        ShapedValue::Number(number) => Json::Number(number.clone()),
        ShapedValue::String(text) => Json::Text(text.to_owned()),
        ShapedValue::Array(items) => Json::List(items.iter().map(decisions_json).collect()),
        ShapedValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.to_owned(), decisions_json(item)))
                .collect(),
        ),
    }
}
