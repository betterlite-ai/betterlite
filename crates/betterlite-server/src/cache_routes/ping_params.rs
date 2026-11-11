use betterlite_cache::configuration::cache::{Backend, Cache};
use betterlite_cache::types::cache_type::CacheMode;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_cache::types::ping::{HealthCheckCacheParams, TextOrNumber};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use serde::Serialize;
use serde_json::Number;

use super::live::wire_name;
use super::masking::redis_kwargs;

// LiteLLM's vars(litellm.cache) minus the "cache" backend; none of these field names is a credential.
#[derive(Serialize)]
struct CacheParams {
    supported_call_types: Vec<&'static str>,
    #[serde(rename = "type")]
    cache_type: &'static str,
    namespace: Option<String>,
    redis_flush_size: Option<usize>,
    ttl: Option<f64>,
    mode: &'static str,
    semantic_cache_scope: &'static str,
}

#[must_use]
pub fn masked_cache_params(cache: &Cache) -> String {
    let params = CacheParams {
        supported_call_types: cache
            .supported_call_types
            .iter()
            .map(|call_type| call_type_name(*call_type))
            .collect(),
        cache_type: wire_name(cache.cache_type),
        namespace: cache.namespace.clone(),
        redis_flush_size: cache.redis_flush_size,
        ttl: cache.ttl,
        mode: mode_name(cache.mode),
        semantic_cache_scope: scope_name(cache.semantic_cache_scope),
    };
    serde_json::to_string(&params).unwrap_or_default()
}

// LiteLLM's HealthCheckCacheParams(**vars(litellm.cache.cache)).model_dump(), masked.
#[must_use]
pub fn health_check_params(cache: &Cache, overrides: &RedisOverrides) -> HealthCheckCacheParams {
    let Backend::Redis(redis) = &cache.backend else {
        return HealthCheckCacheParams {
            host: None,
            port: None,
            redis_kwargs: None,
            namespace: None,
            redis_version: None,
        };
    };
    HealthCheckCacheParams {
        host: None,
        port: None,
        redis_kwargs: Some(redis_kwargs(overrides)),
        namespace: cache.namespace.clone(),
        redis_version: Some(TextOrNumber::Number(Number::from(
            redis.redis_major_version,
        ))),
    }
}

const fn mode_name(mode: CacheMode) -> &'static str {
    match mode {
        CacheMode::DefaultOn => "default_on",
        CacheMode::DefaultOff => "default_off",
    }
}

const fn scope_name(scope: SemanticCacheScope) -> &'static str {
    match scope {
        SemanticCacheScope::Key => "key",
        SemanticCacheScope::EndUser => "end_user",
    }
}

const fn call_type_name(call_type: CachingSupportedCallType) -> &'static str {
    match call_type {
        CachingSupportedCallType::Completion => "completion",
        CachingSupportedCallType::Acompletion => "acompletion",
        CachingSupportedCallType::Embedding => "embedding",
        CachingSupportedCallType::Aembedding => "aembedding",
        CachingSupportedCallType::Atranscription => "atranscription",
        CachingSupportedCallType::Transcription => "transcription",
        CachingSupportedCallType::AtextCompletion => "atext_completion",
        CachingSupportedCallType::TextCompletion => "text_completion",
        CachingSupportedCallType::Arerank => "arerank",
        CachingSupportedCallType::Rerank => "rerank",
        CachingSupportedCallType::Responses => "responses",
        CachingSupportedCallType::Aresponses => "aresponses",
        CachingSupportedCallType::AnthropicMessages => "anthropic_messages",
        CachingSupportedCallType::AanthropicMessages => "aanthropic_messages",
    }
}

mod tests;
