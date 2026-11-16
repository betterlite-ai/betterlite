use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde_json::Number;

use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, TokenDetails, Usage,
};

pub fn record_usage_only_chunk(shaper: &mut ChunkShaper, mut model_response: StreamChunk) {
    if model_response.usage.is_none() {
        return;
    }
    model_response.choices.clear();
    shaper.chunks.push(model_response);
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
    let Some(reporting) = provider else {
        return;
    };
    if !usage_cost_providers.iter().any(|known| known == reporting) {
        return;
    }
    let reported = response
        .usage
        .as_ref()
        .and_then(|usage| usage.cost.as_ref())
        .and_then(resolve_provider_reported_cost);
    let Some(amount) = reported else {
        return;
    };
    let Ok(number) = Number::from_str(&amount.normalize().to_string()) else {
        return;
    };
    let entry = response
        .hidden_params
        .entry("additional_headers".to_owned())
        .or_insert_with(|| JsonValue::Object(JsonObject::new()));
    let JsonValue::Object(fields) = entry else {
        return;
    };
    fields.insert(
        "llm_provider-x-litellm-response-cost".to_owned(),
        JsonValue::Number(number),
    );
}

// PromptTokensDetailsWrapper resolves cache_write_tokens first and mirrors it into cache_creation_tokens.
#[must_use]
pub fn coerce_token_details(details: Option<&TokenDetails>) -> Option<TokenDetails> {
    let counted = details?;
    let nested = counted
        .extra
        .get("cache_creation_input_tokens")
        .and_then(JsonValue::as_i64);
    let write = counted
        .cache_write_tokens
        .or(counted.cache_creation_tokens)
        .or(nested);
    let coerced = TokenDetails {
        cached_tokens: counted.cached_tokens,
        cache_write_tokens: write,
        cache_creation_tokens: write,
        cache_creation_token_details: counted.cache_creation_token_details.clone(),
        extra: counted.extra.clone(),
    };
    Some(coerced)
}

#[must_use]
pub fn calculate_total_usage(chunks: &[StreamChunk]) -> Usage {
    let mut prompt_tokens: i64 = 0;
    let mut completion_tokens: i64 = 0;
    let mut latest: Option<&Usage> = None;
    let mut prompt_details: Option<TokenDetails> = None;
    let mut completion_details: Option<TokenDetails> = None;
    let mut cache_creation: Option<JsonValue> = None;
    for usage in chunks.iter().filter_map(|chunk| chunk.usage.as_ref()) {
        latest = Some(usage);
        prompt_tokens = usage.prompt_tokens;
        completion_tokens = usage.completion_tokens;
        let incoming_prompt = coerce_token_details(usage.prompt_tokens_details.as_ref());
        if let Some(nested) = incoming_prompt
            .as_ref()
            .and_then(|counted| counted.cache_creation_token_details.clone())
        {
            cache_creation = Some(nested);
        }
        if incoming_prompt.is_some() {
            prompt_details = incoming_prompt;
        }
        let incoming_completion = coerce_token_details(usage.completion_tokens_details.as_ref());
        if incoming_completion.is_some() {
            completion_details = incoming_completion;
        }
    }
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        cost: latest.and_then(|usage| usage.cost.clone()),
        prompt_tokens_details: attach_cache_creation(prompt_details, cache_creation),
        completion_tokens_details: completion_details,
        extra: JsonObject::new(),
    }
}

fn attach_cache_creation(
    details: Option<TokenDetails>,
    cache_creation: Option<JsonValue>,
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
