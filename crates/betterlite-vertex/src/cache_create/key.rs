use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::request::{CacheKeyRequest, KeyParam};
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use serde::Serialize;

use super::error::CacheCreateError;

// LiteLLM's context-caching cache exists only to derive a key, with no namespace and provider params off
const LOCAL_KEY_SETTINGS: CacheKeySettings = CacheKeySettings {
    cache_type: LiteLlmCacheType::Local,
    mode: CacheMode::DefaultOn,
    namespace: None,
    semantic_cache_scope: SemanticCacheScope::Key,
    include_provider_params: false,
};

// B4296's Cache.get_cache_key: the four request arguments in the order LiteLLM passes them, so the material matches
pub(crate) fn context_cache_key<L, C>(
    messages: &[Message],
    tools: Option<&[Json]>,
    tool_choice: Option<&Json>,
    model: &str,
    hooks: &VerboseHooks<L, C>,
) -> Result<String, CacheCreateError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let mut request = CacheKeyRequest {
        params: vec![
            KeyParam {
                name: "messages".to_owned(),
                value: key_value(messages)?,
            },
            KeyParam {
                name: "tools".to_owned(),
                value: key_value(&tools)?,
            },
            KeyParam {
                name: "tool_choice".to_owned(),
                value: key_value(&tool_choice)?,
            },
            KeyParam {
                name: "model".to_owned(),
                value: KeyValue::String(model.to_owned()),
            },
        ],
        metadata: None,
        litellm_metadata: None,
        litellm_params: None,
        cache_control: None,
    };
    get_cache_key(&mut request, &LOCAL_KEY_SETTINGS, hooks)
        .map_err(|source| CacheCreateError::Key { source })
}

fn key_value<T>(value: &T) -> Result<KeyValue, CacheCreateError>
where
    T: Serialize + ?Sized,
{
    let encoded =
        serde_json::to_vec(value).map_err(|source| CacheCreateError::Encode { source })?;
    serde_json::from_slice(&encoded).map_err(|source| CacheCreateError::Encode { source })
}
