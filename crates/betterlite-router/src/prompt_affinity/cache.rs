use async_trait::async_trait;
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::json::text;
use super::prefix::{PrefixError, lookback_keys, prefix_positions, prompt_caching_cache_key};

// PromptCachingCache.PROMPT_CACHE_PIN_TTL_SECONDS
pub const PIN_TTL_SECONDS: f64 = 300.0;

// LiteLLM's router DualCache as the pin store: writes and reads log a backend failure and carry on
#[async_trait]
pub trait PinCache: Send + Sync {
    async fn store(&self, key: &str, value: &Json);
    async fn read(&self, keys: &[String]) -> Vec<Option<Json>>;
}

// PromptCachingCache.async_add_model_id: pin the last cacheable block to the deployment that served it
pub async fn store_pin(
    pins: &dyn PinCache,
    model_id: &str,
    messages: Option<&[Json]>,
    tools: Option<&[Json]>,
    truncator: &Base64Truncator,
) -> Result<(), PrefixError> {
    let Some(cache_key) = prompt_caching_cache_key(messages, tools, truncator)? else {
        return Ok(());
    };
    let mut value = IndexMap::new();
    value.insert("model_id".to_owned(), Json::Text(model_id.to_owned()));
    pins.store(&cache_key, &Json::Object(value)).await;
    Ok(())
}

// PromptCachingCache.async_get_model_id: the first pin on the walk back from the last cacheable block
pub async fn pinned_model_id(
    pins: &dyn PinCache,
    messages: Option<&[Json]>,
    tools: Option<&[Json]>,
    truncator: &Base64Truncator,
) -> Result<Option<String>, PrefixError> {
    let cache_keys = lookback_keys(&prefix_positions(messages, tools, truncator)?);
    if cache_keys.is_empty() {
        return Ok(None);
    }
    // PromptCachingCache._first_pin: a value without a string model_id is not a pin
    Ok(pins
        .read(&cache_keys)
        .await
        .iter()
        .flatten()
        .find_map(model_id_of))
}

fn model_id_of(value: &Json) -> Option<String> {
    match value {
        Json::Object(fields) => fields.get("model_id").and_then(text).map(str::to_owned),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
