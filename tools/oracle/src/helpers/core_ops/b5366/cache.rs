use betterlite_core::message_utilities::prompt_cache_checks::{
    PromptCacheRequest, PromptCacheSettings, get_prompt_cache_min_tokens, is_cached_message,
    is_prompt_caching_valid_prompt,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use super::fixture::Session;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    IsCachedMessage(CachedMessageArgs),
    GetPromptCacheMinTokens(MinTokensArgs),
    IsPromptCachingValidPrompt(ValidPromptArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct CachedMessageArgs {
    message: Json,
    #[serde(default)]
    disable_transform: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MinTokensArgs {
    model: String,
    #[serde(default)]
    minimum_token_override: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ValidPromptArgs {
    model: String,
    #[serde(default)]
    messages: Option<Vec<Json>>,
    #[serde(default)]
    tools: Option<Vec<Json>>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    min_token_count: Option<u64>,
}

pub(super) async fn run(call: &Call, session: &Session) -> serde_json::Value {
    match call {
        Call::IsCachedMessage(args) => {
            json!(is_cached_message(&args.message, args.disable_transform))
        }
        Call::GetPromptCacheMinTokens(args) => {
            let settings = PromptCacheSettings {
                minimum_token_override: args.minimum_token_override,
            };
            json!(get_prompt_cache_min_tokens(
                &args.model,
                settings,
                session.catalog.as_ref()
            ))
        }
        Call::IsPromptCachingValidPrompt(args) => {
            json!(valid_prompt(args, session).await)
        }
    }
}

async fn valid_prompt(args: &ValidPromptArgs, session: &Session) -> bool {
    let request = PromptCacheRequest {
        model: &args.model,
        messages: args.messages.as_deref(),
        tools: args.tools.as_deref(),
        custom_llm_provider: args.custom_llm_provider.as_deref(),
        min_token_count: args.min_token_count,
    };
    is_prompt_caching_valid_prompt(
        &request,
        PromptCacheSettings::default(),
        &session.counter,
        session.catalog.as_ref(),
    )
    .await
}
