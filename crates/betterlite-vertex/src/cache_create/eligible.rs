use betterlite_core::message_utilities::prompt_cache_checks::{
    PromptCacheRequest, PromptCacheSettings, is_prompt_caching_valid_prompt,
};
use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;

use crate::cache_lookup::target::CacheProvider;

use super::orchestration::{CacheCreateOptions, CacheCreateRequest};

// LiteLLM asks is_prompt_caching_valid_prompt for the cached block plus the tools, with the provider prefix on the model
pub(crate) async fn is_cacheable<F>(
    request: &CacheCreateRequest<'_, F>,
    options: &CacheCreateOptions,
    cached: &[Message],
) -> bool
where
    F: RemoteMediaFetcher + Sync,
{
    let Ok(encoded) = serde_json::to_vec(cached) else {
        return false;
    };
    let Ok(messages) = serde_json::from_slice::<Vec<Json>>(&encoded) else {
        return false;
    };
    is_prompt_caching_valid_prompt(
        &PromptCacheRequest {
            model: request.model,
            messages: Some(&messages),
            tools: options.tools.as_deref(),
            custom_llm_provider: Some(provider_name(request.provider)),
            min_token_count: None,
        },
        PromptCacheSettings {
            minimum_token_override: request.minimum_token_override,
        },
        request.counter,
        request.catalog,
    )
    .await
}

const fn provider_name(provider: CacheProvider) -> &'static str {
    match provider {
        CacheProvider::Vertex => "vertex_ai",
        CacheProvider::VertexBeta => "vertex_ai_beta",
        CacheProvider::Gemini => "gemini",
    }
}
