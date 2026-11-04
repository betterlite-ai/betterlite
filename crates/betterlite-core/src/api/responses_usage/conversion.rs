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
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::{ResponsesUsageError, Usage, typed};

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

pub(super) fn from_response_usage(typed: &ResponseApiUsage) -> Result<Usage, ResponsesUsageError> {
    let prompt_tokens = typed.input_tokens;
    let completion_tokens = typed.output_tokens;
    let prompt_details = typed
        .input_tokens_details
        .as_ref()
        .map(prompt_details)
        .transpose()?
        .flatten();
    let completion_details = typed
        .output_tokens_details
        .as_ref()
        .map(|details| completion_details(details, completion_tokens))
        .transpose()?;
    let extensions = extension_fields(&typed.extra)?;
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

fn prompt_details(
    details: &InputTokensDetails,
) -> Result<Option<PromptTokensDetails>, serde_json::Error> {
    let value = serde_json::to_value(details)?;
    let fields = Map::from_iter([("prompt_tokens_details".to_owned(), value)]);
    ordinary_details(&fields).map(|(prompt, _)| prompt)
}

pub(super) fn ordinary_details(
    fields: &Map<String, Value>,
) -> Result<(Option<PromptTokensDetails>, Option<CompletionTokensDetails>), serde_json::Error> {
    let prompt = ordinary_detail_map(fields, "prompt_tokens_details")?
        .filter(|details| !details.is_empty())
        .map(|details| prompt_details_from_map(&details))
        .transpose()?;
    let completion = ordinary_detail_map(fields, "completion_tokens_details")?
        .filter(|details| !details.is_empty())
        .map(|details| completion_details_from_map(&details))
        .transpose()?;
    Ok((prompt, completion))
}

fn prompt_details_from_map(
    details: &Map<String, Value>,
) -> Result<PromptTokensDetails, serde_json::Error> {
    let resolved_cache_tokens = lax_integer_field(details.get("cache_write_tokens"))?
        .or(lax_integer_field(details.get("cache_creation_tokens"))?);
    let cached_tokens_details = match details.get("cached_tokens_details") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let nested = serde_json::from_value::<Map<String, Value>>(value.clone())?;
            Some(CachedTokensDetails {
                text_tokens: lax_integer_field(nested.get("text_tokens"))?,
                audio_tokens: lax_integer_field(nested.get("audio_tokens"))?,
                image_tokens: lax_integer_field(nested.get("image_tokens"))?,
            })
        }
    };
    Ok(PromptTokensDetails {
        audio_tokens: lax_integer_field(details.get("audio_tokens"))?,
        cached_tokens: lax_integer_field(details.get("cached_tokens"))?,
        cached_tokens_details,
        text_tokens: lax_integer_field(details.get("text_tokens"))?,
        image_tokens: lax_integer_field(details.get("image_tokens"))?,
        video_tokens: lax_integer_field(details.get("video_tokens"))?,
        web_search_requests: lax_integer_field(details.get("web_search_requests"))?,
        google_maps_grounding_requests: lax_integer_field(
            details.get("google_maps_grounding_requests"),
        )?,
        tool_use_tokens: lax_integer_field(details.get("tool_use_tokens"))?,
        character_count: lax_integer_field(details.get("character_count"))?,
        image_count: lax_integer_field(details.get("image_count"))?,
        video_length_seconds: optional_decimal(details, "video_length_seconds")?,
        audio_length_seconds: optional_decimal(details, "audio_length_seconds")?,
        query_count: lax_integer_field(details.get("query_count"))?,
        cache_write_tokens: resolved_cache_tokens,
        cache_creation_tokens: resolved_cache_tokens,
    })
}

fn completion_details_from_map(
    details: &Map<String, Value>,
) -> Result<CompletionTokensDetails, serde_json::Error> {
    Ok(CompletionTokensDetails {
        text_tokens: lax_integer_field(details.get("text_tokens"))?,
        audio_tokens: lax_integer_field(details.get("audio_tokens"))?,
        image_tokens: lax_integer_field(details.get("image_tokens"))?,
        video_tokens: lax_integer_field(details.get("video_tokens"))?,
        reasoning_tokens: lax_integer_field(details.get("reasoning_tokens"))?,
        accepted_prediction_tokens: lax_integer_field(details.get("accepted_prediction_tokens"))?,
        rejected_prediction_tokens: lax_integer_field(details.get("rejected_prediction_tokens"))?,
    })
}

fn ordinary_detail_map(
    fields: &Map<String, Value>,
    name: &str,
) -> Result<Option<Map<String, Value>>, serde_json::Error> {
    let Some(value) = fields.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let details = serde_json::from_value::<Map<String, Value>>(value.clone())?;
    Ok(Some(details))
}

fn optional_decimal(
    fields: &Map<String, Value>,
    name: &str,
) -> Result<Option<rust_decimal::Decimal>, serde_json::Error> {
    fields
        .get(name)
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value::<rust_decimal::Decimal>(value.clone()))
        .transpose()
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

fn lax_integer_field<T>(field: Option<&T>) -> Result<Option<i64>, serde_json::Error>
where
    T: Serialize,
{
    field
        .map(serde_json::to_value)
        .transpose()?
        .as_ref()
        .map(typed::lax_integer)
        .transpose()
        .map(Option::flatten)
}

fn extension_fields(
    extra: &BTreeMap<String, WireJson>,
) -> Result<Map<String, Value>, serde_json::Error> {
    let mut fields = Map::new();
    for (name, field) in extra {
        if !EXCLUDED.contains(&name.as_str()) {
            fields.insert(name.clone(), serde_json::to_value(field)?);
        }
    }
    Ok(fields)
}
