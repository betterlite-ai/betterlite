use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde_json::Number;

use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk};
use crate::stream_chunk_shaping::usage::{CacheCreationTokenDetails, TokenDetails, Usage};

pub fn record_usage_only_chunk(shaper: &mut ChunkShaper, model_response: StreamChunk) {
    if model_response.usage.is_none() {
        return;
    }
    let mut recorded = model_response;
    recorded.choices.clear();
    shaper.chunks.push(recorded);
}

#[must_use]
pub fn resolve_provider_reported_cost(usage_cost: &JsonValue) -> Option<Decimal> {
    match usage_cost {
        JsonValue::Number(number) => {
            let text = number.to_string();
            Decimal::from_str(&text)
                .or_else(|_| Decimal::from_scientific(&text))
                .ok()
        }
        JsonValue::Object(fields) => fields
            .get("total_cost")
            .and_then(resolve_provider_reported_cost),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::String(_) | JsonValue::Array(_) => None,
    }
}

pub fn propagate_usage_cost(
    response: &mut StreamChunk,
    provider: Option<&str>,
    usage_cost_providers: &[String],
) {
    let Some(known_provider) = provider else {
        return;
    };
    if !usage_cost_providers
        .iter()
        .any(|known| known == known_provider)
    {
        return;
    }
    let cost = response
        .usage
        .as_ref()
        .and_then(|usage| usage.cost.as_ref())
        .and_then(resolve_provider_reported_cost);
    let Some(reported) = cost else {
        return;
    };
    let Ok(number) = Number::from_str(&reported.normalize().to_string()) else {
        return;
    };
    let headers = response
        .hidden_params
        .entry("additional_headers".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonObject::new()));
    if let JsonValue::Object(fields) = headers {
        fields.insert(
            "llm_provider-x-litellm-response-cost".to_owned(),
            JsonValue::Number(number),
        );
    }
}

#[must_use]
pub fn cache_token_count(details: Option<&TokenDetails>, keys: &[&str]) -> i64 {
    let Some(counted) = details else {
        return 0;
    };
    for key in keys {
        let value = match *key {
            "cached_tokens" => counted.cached_tokens,
            "cache_write_tokens" => counted.cache_write_tokens,
            "cache_creation_tokens" => counted.cache_creation_tokens,
            _ => None,
        };
        if let Some(count) = value.filter(|count| *count != 0) {
            return count;
        }
    }
    0
}

#[must_use]
pub fn coerce_token_details(details: Option<&TokenDetails>) -> Option<TokenDetails> {
    let mut coerced = details?.clone();
    let nested = coerced
        .extra
        .get("cache_creation_input_tokens")
        .and_then(JsonValue::as_i64);
    let write = coerced
        .cache_write_tokens
        .or(coerced.cache_creation_tokens)
        .or(nested);
    let creation = coerced.cache_creation_tokens.or(write);
    coerced.cache_write_tokens = write;
    coerced.cache_creation_tokens = creation;
    Some(coerced)
}

pub fn backfill_cache_usage(usage: &mut Usage) {
    let details = coerce_token_details(usage.prompt_tokens_details.as_ref());
    let cached = cache_token_count(details.as_ref(), &["cached_tokens"]);
    let creation = cache_token_count(
        details.as_ref(),
        &["cache_write_tokens", "cache_creation_tokens"],
    );
    if usage
        .extra
        .get("cache_read_input_tokens")
        .is_none_or(|value| matches!(value, JsonValue::Null))
    {
        usage.extra.insert(
            "cache_read_input_tokens".to_owned(),
            JsonValue::Number(Number::from(cached)),
        );
    }
    if usage
        .extra
        .get("cache_creation_input_tokens")
        .is_none_or(|value| matches!(value, JsonValue::Null))
    {
        usage.extra.insert(
            "cache_creation_input_tokens".to_owned(),
            JsonValue::Number(Number::from(creation)),
        );
    }
    if usage.prompt_tokens_details.is_none() {
        usage.prompt_tokens_details = Some(zero_cached_details());
    }
}

const fn zero_cached_details() -> TokenDetails {
    TokenDetails {
        cached_tokens: Some(0),
        audio_tokens: None,
        text_tokens: None,
        image_tokens: None,
        video_tokens: None,
        accepted_prediction_tokens: None,
        rejected_prediction_tokens: None,
        reasoning_tokens: None,
        web_search_requests: None,
        google_maps_grounding_requests: None,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        query_count: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cache_creation_token_details: None,
        cached_tokens_details: None,
        extra: JsonObject::new(),
    }
}

#[must_use]
pub fn calculate_total_usage(chunks: &[StreamChunk]) -> Usage {
    let mut prompt_tokens: i64 = 0;
    let mut completion_tokens: i64 = 0;
    let mut latest: Option<&Usage> = None;
    let mut prompt_details: Option<TokenDetails> = None;
    let mut completion_details: Option<TokenDetails> = None;
    let mut cache_creation: Option<CacheCreationTokenDetails> = None;
    for chunk in chunks {
        let Some(usage) = chunk.usage.as_ref() else {
            continue;
        };
        latest = Some(usage);
        prompt_tokens = usage.prompt_tokens;
        completion_tokens = usage.completion_tokens;
        absorb_details(
            usage,
            &mut prompt_details,
            &mut completion_details,
            &mut cache_creation,
        );
    }
    let mut returned = Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        cost: None,
        prompt_tokens_details: attach_cache_creation(prompt_details, cache_creation),
        completion_tokens_details: completion_details,
        extra: JsonObject::new(),
    };
    if let Some(cost) = latest.and_then(|usage| usage.cost.clone()) {
        returned.cost = Some(cost);
    }
    returned
}

fn absorb_details(
    usage: &Usage,
    prompt_details: &mut Option<TokenDetails>,
    completion_details: &mut Option<TokenDetails>,
    cache_creation: &mut Option<CacheCreationTokenDetails>,
) {
    let incoming_prompt = coerce_token_details(usage.prompt_tokens_details.as_ref());
    let incoming_creation = incoming_prompt
        .as_ref()
        .and_then(|incoming| incoming.cache_creation_token_details.as_ref());
    if let Some(details) = incoming_creation {
        *cache_creation = Some(details.clone());
    }
    if incoming_prompt.is_some() {
        *prompt_details = incoming_prompt;
    }
    let incoming_completion = coerce_token_details(usage.completion_tokens_details.as_ref());
    if incoming_completion.is_some() {
        *completion_details = incoming_completion;
    }
}

fn attach_cache_creation(
    details: Option<TokenDetails>,
    cache_creation: Option<CacheCreationTokenDetails>,
) -> Option<TokenDetails> {
    let Some(cache) = cache_creation else {
        return details;
    };
    let mut counted = details?;
    if counted.cache_creation_token_details.is_none() {
        counted.cache_creation_token_details = Some(cache);
    }
    Some(counted)
}
