use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use uuid::Uuid;

use crate::legacy_prediction::projection::{LegacyResult, LegacyUsage};

// LiteLLM returns the whole ModelResponse for a code model's fake stream; its dump shape is pinned field by field
pub(crate) fn model_response(result: &LegacyResult, content: Option<&str>) -> Json {
    let response = IndexMap::from([
        (
            String::from("id"),
            Json::Text(format!("chatcmpl-{}", Uuid::new_v4())),
        ),
        (
            String::from("created"),
            Json::Number(Number::from(result.created)),
        ),
        (String::from("model"), Json::Text(result.model.clone())),
        (
            String::from("object"),
            Json::Text(String::from("chat.completion")),
        ),
        (String::from("system_fingerprint"), Json::Null),
        (
            String::from("choices"),
            Json::List(vec![Json::Object(choice(content))]),
        ),
        (String::from("usage"), usage(&result.usage)),
    ]);
    Json::Object(response)
}

// Choices' "stop" default, index 0 and a Message whose unset tool fields stay null in the dump
fn choice(content: Option<&str>) -> IndexMap<String, Json> {
    let text = content.map_or(Json::Null, |text| Json::Text(text.to_owned()));
    let message = IndexMap::from([
        (String::from("content"), text),
        (String::from("role"), Json::Text(String::from("assistant"))),
        (String::from("tool_calls"), Json::Null),
        (String::from("function_call"), Json::Null),
        (String::from("provider_specific_fields"), Json::Null),
    ]);
    IndexMap::from([
        (
            String::from("finish_reason"),
            Json::Text(String::from("stop")),
        ),
        (String::from("index"), Json::Number(Number::from(0_u8))),
        (String::from("message"), Json::Object(message)),
    ])
}

// Usage next to the ModelResponse dump: the two detail objects stay null until a provider fills them
fn usage(usage: &LegacyUsage) -> Json {
    Json::Object(IndexMap::from([
        (
            String::from("completion_tokens"),
            Json::Number(Number::from(usage.completion_tokens)),
        ),
        (
            String::from("prompt_tokens"),
            Json::Number(Number::from(usage.prompt_tokens)),
        ),
        (
            String::from("total_tokens"),
            Json::Number(Number::from(usage.total_tokens)),
        ),
        (String::from("completion_tokens_details"), Json::Null),
        (String::from("prompt_tokens_details"), Json::Null),
    ]))
}
