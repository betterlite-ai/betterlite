use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use thiserror::Error;

use super::catalog_lookup;
use super::json;

pub const DEFAULT_MIN_CACHE_TOKENS: u64 = 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct PromptCacheSettings {
    pub minimum_token_override: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PromptCacheRequest<'request> {
    pub model: &'request str,
    pub messages: Option<&'request [Json]>,
    pub tools: Option<&'request [Json]>,
    pub custom_llm_provider: Option<&'request str>,
    pub min_token_count: Option<u64>,
}

#[derive(Debug, Error)]
pub enum PromptCacheError {
    #[error("the messages could not be read as token counter messages")]
    Messages(#[source] serde_json::Error),
    #[error("the token count failed")]
    Count(#[from] CountError),
    #[error("the token count does not fit")]
    CountOverflow,
}

#[must_use]
pub fn is_cached_message(message: &Json, disable_transform: bool) -> bool {
    if disable_transform {
        return false;
    }
    let Some(fields) = json::object(message) else {
        return false;
    };
    if is_ephemeral(fields.get("cache_control")) {
        return true;
    }
    let Some(Json::List(content)) = fields.get("content") else {
        return false;
    };
    content.iter().any(|item| {
        json::object(item).is_some_and(|part| {
            part.get("type").and_then(json::text) == Some("text")
                && is_ephemeral(part.get("cache_control"))
        })
    })
}

fn is_ephemeral(cache_control: Option<&Json>) -> bool {
    cache_control
        .and_then(json::object)
        .and_then(|fields| fields.get("type"))
        .and_then(json::text)
        == Some("ephemeral")
}

// the cost map's field is the source of truth, and an unknown model never fails this lookup
#[must_use]
pub fn get_prompt_cache_min_tokens(
    model: &str,
    settings: PromptCacheSettings,
    catalog: &ModelCatalog,
) -> u64 {
    if let Some(override_tokens) = settings.minimum_token_override {
        return override_tokens;
    }
    catalog_lookup::resolved(catalog, model)
        .and_then(|entry| entry.fields.metadata.prompt_cache_min_tokens)
        .unwrap_or(DEFAULT_MIN_CACHE_TOKENS)
}

pub async fn is_prompt_caching_valid_prompt(
    request: &PromptCacheRequest<'_>,
    settings: PromptCacheSettings,
    counter: &Counter,
    catalog: &ModelCatalog,
) -> bool {
    match valid_prompt(request, settings, counter, catalog).await {
        Ok(valid) => valid,
        Err(error) => {
            tracing::debug!(%error, "prompt caching validity check failed");
            false
        }
    }
}

async fn valid_prompt(
    request: &PromptCacheRequest<'_>,
    settings: PromptCacheSettings,
    counter: &Counter,
    catalog: &ModelCatalog,
) -> Result<bool, PromptCacheError> {
    let mut model = request.model.to_owned();
    if let Some(provider) = request.custom_llm_provider
        && !model.starts_with(provider)
    {
        model = format!("{provider}/{model}");
    }
    let messages = match request.messages {
        Some(found) => {
            let value = serde_json::to_value(found).map_err(PromptCacheError::Messages)?;
            let parsed: Vec<Message> =
                serde_json::from_value(value).map_err(PromptCacheError::Messages)?;
            Some(parsed)
        }
        None => None,
    };
    let token_request = TokenRequest {
        model: &model,
        custom_tokenizer: None,
        text: None,
        messages: messages.as_deref(),
        count_response_tokens: Some(false),
        tools: request.tools,
        tool_choice: None,
    };
    let tokens = u64::try_from(counter.count(&token_request).await?)
        .map_err(|_conversion| PromptCacheError::CountOverflow)?;
    let minimum = request
        .min_token_count
        .unwrap_or_else(|| get_prompt_cache_min_tokens(&model, settings, catalog));
    Ok(tokens >= minimum)
}
