use betterlite_models::decisions::Json as WireJson;
use betterlite_models::modality_cost::{
    completion::text_without_nested_reasoning, error::ModalityCostError,
};
use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails,
};
use betterlite_openai::api::responses_objects::{
    InputTokensDetails, OutputTokensDetails, ResponseApiUsage,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::{ResponsesUsageError, Usage};

const EXCLUDED: [&str; 11] = [
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "input_tokens_details",
    "output_tokens_details",
    "input_token_details",
    "output_token_details",
    "prompt_tokens",
    "completion_tokens",
    "prompt_tokens_details",
    "completion_tokens_details",
];

pub(super) fn from_response_usage(typed: ResponseApiUsage) -> Result<Usage, ResponsesUsageError> {
    let prompt_tokens = typed.input_tokens;
    let completion_tokens = typed.output_tokens;
    let prompt_details = typed.input_tokens_details.as_ref().map(prompt_details);
    let completion_details = typed
        .output_tokens_details
        .as_ref()
        .map(|details| completion_details(details, completion_tokens))
        .transpose()?;
    let extensions = extension_fields(typed.extra)?;
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        prompt_tokens_details: prompt_details,
        completion_tokens_details: completion_details,
        cost: typed.cost,
        extensions,
    })
}

fn prompt_details(details: &InputTokensDetails) -> PromptTokensDetails {
    PromptTokensDetails {
        audio_tokens: details.audio_tokens,
        cached_tokens: Some(details.cached_tokens),
        cached_tokens_details: details.cached_tokens_details.as_ref().map(|cached| {
            CachedTokensDetails {
                text_tokens: cached.text_tokens,
                audio_tokens: cached.audio_tokens,
                image_tokens: cached.image_tokens,
            }
        }),
        text_tokens: details.text_tokens,
        image_tokens: details.image_tokens,
        video_tokens: details.video_tokens,
        web_search_requests: detail_integer(&details.extra, "web_search_requests"),
        google_maps_grounding_requests: detail_integer(
            &details.extra,
            "google_maps_grounding_requests",
        ),
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: detail_integer(&details.extra, "cache_write_tokens"),
        cache_creation_tokens: None,
    }
}

fn completion_details(
    details: &OutputTokensDetails,
    completion: i64,
) -> Result<CompletionTokensDetails, ModalityCostError> {
    let reasoning = details.reasoning_tokens;
    let audio = details.audio_tokens;
    let image = detail_integer(&details.extra, "image_tokens");
    let text = details
        .text_tokens
        .map(|reported| {
            text_without_nested_reasoning(
                completion,
                reported,
                reasoning.unwrap_or(0),
                audio.unwrap_or(0).saturating_add(image.unwrap_or(0)),
            )
        })
        .transpose()?;
    Ok(CompletionTokensDetails {
        text_tokens: text,
        audio_tokens: audio,
        image_tokens: image,
        video_tokens: None,
        reasoning_tokens: reasoning,
        accepted_prediction_tokens: None,
        rejected_prediction_tokens: None,
    })
}

fn detail_integer(details: &BTreeMap<String, WireJson>, field: &str) -> Option<i64> {
    match details.get(field)? {
        WireJson::Bool(value) => Some(i64::from(*value)),
        WireJson::Number(value) => value.as_i64(),
        WireJson::Null | WireJson::Text(_) | WireJson::List(_) | WireJson::Object(_) => None,
    }
}

fn extension_fields(
    extra: BTreeMap<String, WireJson>,
) -> Result<Map<String, Value>, serde_json::Error> {
    extra
        .into_iter()
        .filter(|(key, _)| !EXCLUDED.contains(&key.as_str()))
        .map(|(name, field)| serde_json::to_value(field).map(|json| (name, json)))
        .collect()
}
