use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageNumber};
use betterlite_vertex::gemini_candidates::choice::{Choice, Choices, Message};
use betterlite_vertex::gemini_response::complete::{CompleteMetadata, CompleteResponse};
use indexmap::IndexMap;
use serde_json::Number;

use super::VertexPassthroughError;

// model_dump() on LiteLLM's Message drops the fields the transform never set; these four stay null in Rust.
const UNSET_MESSAGE_FIELDS: [&str; 4] = [
    "reasoning_content",
    "audio",
    "annotations",
    "provider_specific_fields",
];

// LiteLLM logs ModelResponse.model_dump(): the served model, its candidates, usage and the metadata lists.
pub(super) fn model_response_body(
    response: &CompleteResponse,
    created: i64,
) -> Result<Json, serde_json::Error> {
    let mut fields = IndexMap::new();
    fields.insert("id".to_owned(), Json::Text(response.id.clone()));
    fields.insert(
        "object".to_owned(),
        Json::Text("chat.completion".to_owned()),
    );
    fields.insert("created".to_owned(), Json::Number(Number::from(created)));
    fields.insert("model".to_owned(), Json::Text(response.model.clone()));
    fields.insert("system_fingerprint".to_owned(), Json::Null);
    fields.insert("choices".to_owned(), choices_json(&response.choices)?);
    fields.insert("usage".to_owned(), usage_json(&response.usage));
    if let Some(tier) = &response.service_tier {
        fields.insert("service_tier".to_owned(), Json::Text(tier.clone()));
    }
    insert_metadata_lists(&mut fields, response.metadata.as_ref());
    Ok(Json::Object(fields))
}

pub(super) fn set_body_id(body: &mut Json, call_id: &str) -> Result<(), VertexPassthroughError> {
    let Json::Object(fields) = body else {
        return Err(VertexPassthroughError::Shape {
            kind: "chat response",
        });
    };
    fields.insert("id".to_owned(), Json::Text(call_id.to_owned()));
    Ok(())
}

// litellm.ModelResponse() with no body: its one default choice and the fields the constructor always sets.
pub(super) fn empty_chat_response(created: i64) -> Json {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    message.insert("content".to_owned(), Json::Null);
    message.insert("tool_calls".to_owned(), Json::Null);
    message.insert("function_call".to_owned(), Json::Null);
    let mut choice = IndexMap::new();
    choice.insert("finish_reason".to_owned(), Json::Text("stop".to_owned()));
    choice.insert("index".to_owned(), Json::Number(Number::from(0_i64)));
    choice.insert("message".to_owned(), Json::Object(message));
    let mut fields = IndexMap::new();
    fields.insert("id".to_owned(), Json::Text(String::new()));
    fields.insert(
        "object".to_owned(),
        Json::Text("chat.completion".to_owned()),
    );
    fields.insert("created".to_owned(), Json::Number(Number::from(created)));
    fields.insert("model".to_owned(), Json::Null);
    fields.insert("system_fingerprint".to_owned(), Json::Null);
    fields.insert("choices".to_owned(), Json::List(vec![Json::Object(choice)]));
    Json::Object(fields)
}

fn insert_metadata_lists(fields: &mut IndexMap<String, Json>, metadata: Option<&CompleteMetadata>) {
    for (name, values) in [
        (
            "vertex_ai_grounding_metadata",
            metadata.map(|found| &found.grounding_metadata),
        ),
        (
            "vertex_ai_url_context_metadata",
            metadata.map(|found| &found.url_context_metadata),
        ),
        (
            "vertex_ai_safety_results",
            metadata.map(|found| &found.safety_results),
        ),
        (
            "vertex_ai_citation_metadata",
            metadata.map(|found| &found.citation_metadata),
        ),
    ] {
        fields.insert(
            name.to_owned(),
            Json::List(values.cloned().unwrap_or_default()),
        );
    }
}

// The provider's raw finish reason rides the choice's provider_specific_fields beside the mapped one.
fn choices_json(choices: &Choices) -> Result<Json, serde_json::Error> {
    let Choices::Complete(complete) = choices else {
        return Ok(Json::List(Vec::new()));
    };
    let mut rows = Vec::with_capacity(complete.len());
    for choice in complete {
        rows.push(choice_json(choice)?);
    }
    Ok(Json::List(rows))
}

fn choice_json(choice: &Choice) -> Result<Json, serde_json::Error> {
    let mut fields = IndexMap::new();
    fields.insert(
        "finish_reason".to_owned(),
        Json::Text(choice.finish_reason.to_owned()),
    );
    fields.insert(
        "index".to_owned(),
        Json::Number(Number::from(i64::from(choice.index))),
    );
    fields.insert("message".to_owned(), message_json(&choice.message)?);
    if let Some(logprobs) = &choice.logprobs {
        fields.insert(
            "logprobs".to_owned(),
            serde_json::from_value(serde_json::to_value(logprobs)?)?,
        );
    }
    if let Some(native) = &choice.native_finish_reason {
        let mut specific = IndexMap::new();
        specific.insert(
            "native_finish_reason".to_owned(),
            Json::Text(native.clone()),
        );
        fields.insert(
            "provider_specific_fields".to_owned(),
            Json::Object(specific),
        );
    }
    Ok(Json::Object(fields))
}

// model_dump() keeps a null only for a field the transform set, so the unset nulls are dropped before logging.
fn message_json(message: &Message) -> Result<Json, serde_json::Error> {
    let mut value: Json = serde_json::from_value(serde_json::to_value(message)?)?;
    if let Json::Object(fields) = &mut value {
        for name in UNSET_MESSAGE_FIELDS {
            if fields.get(name) == Some(&Json::Null) {
                fields.shift_remove(name);
            }
        }
    }
    Ok(value)
}

// LiteLLM's ModelResponse.model_dump() usage block, which the Vertex factory's cost record reads back too.
pub(super) fn usage_json(usage: &Usage) -> Json {
    let mut fields = IndexMap::new();
    insert_counts(
        &mut fields,
        &[
            ("prompt_tokens", Some(usage.prompt_tokens)),
            ("completion_tokens", Some(usage.completion_tokens)),
            ("total_tokens", Some(usage.total_tokens)),
        ],
    );
    if let Some(details) = &usage.prompt_tokens_details {
        let mut values = IndexMap::new();
        insert_counts(&mut values, &prompt_counts(details));
        fields.insert("prompt_tokens_details".to_owned(), Json::Object(values));
        if let Some(cached) = details.cached_tokens {
            fields.insert(
                "cache_read_input_tokens".to_owned(),
                Json::Number(Number::from(cached)),
            );
        }
    }
    if let Some(details) = &usage.completion_tokens_details {
        let mut values = IndexMap::new();
        insert_counts(
            &mut values,
            &[
                ("text_tokens", details.text_tokens),
                ("audio_tokens", details.audio_tokens),
                ("image_tokens", details.image_tokens),
                ("video_tokens", details.video_tokens),
                ("reasoning_tokens", details.reasoning_tokens),
            ],
        );
        fields.insert("completion_tokens_details".to_owned(), Json::Object(values));
    }
    for (name, extra) in &usage.numeric_extras {
        fields.insert(name.clone(), extra_json(extra));
    }
    Json::Object(fields)
}

const fn prompt_counts(details: &PromptTokensDetails) -> [(&'static str, Option<i64>); 7] {
    [
        ("text_tokens", details.text_tokens),
        ("audio_tokens", details.audio_tokens),
        ("image_tokens", details.image_tokens),
        ("video_tokens", details.video_tokens),
        ("web_search_requests", details.web_search_requests),
        (
            "google_maps_grounding_requests",
            details.google_maps_grounding_requests,
        ),
        ("cached_tokens", details.cached_tokens),
    ]
}

fn insert_counts(fields: &mut IndexMap<String, Json>, counts: &[(&str, Option<i64>)]) {
    for (name, value) in counts {
        if let Some(count) = value {
            fields.insert((*name).to_owned(), Json::Number(Number::from(*count)));
        }
    }
}

fn extra_json(extra: &UsageNumber) -> Json {
    match extra {
        UsageNumber::Integer(count) => Json::Number(Number::from(*count)),
        UsageNumber::Boolean(flag) => Json::Bool(*flag),
        UsageNumber::Decimal(value) => decimal_json(*value),
    }
}
