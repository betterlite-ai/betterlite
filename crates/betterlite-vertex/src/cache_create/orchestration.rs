use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_cache::cache_key::derive::get_cache_key;
use betterlite_cache::cache_key::request::{CacheKeyRequest, KeyParam};
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_core::message_utilities::prompt_cache_checks::{
    PromptCacheRequest, PromptCacheSettings, is_prompt_caching_valid_prompt,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::Counter;
use jiff::Timestamp;
use secrecy::SecretString;
use serde::Serialize;

use crate::cache_lookup::lookup::check_cache;
use crate::cache_lookup::target::{CacheProvider, CacheTargetInput, cache_target};
use crate::cache_lookup::transport::CacheTransport;
use crate::cache_prefix::selection::{ends_on_supported_turn, separate_cached_messages};
use crate::cache_prefix::transform::{
    CachedContentRequest, CachedContentRequestBody, cached_content_request,
};
use crate::endpoint_target::target::EndpointTarget;
use crate::model_policy::capabilities::{ModelCapabilityLookup, VertexCapabilityProvider};

use super::error::CacheCreateError;
use super::records::{CREATED_CACHE_LIMIT, CreatedCache, decode_created_cache};
use super::request::{CreateCacheBody, cache_create_request};

// LiteLLM's context-caching cache exists only to derive a key, with no namespace and provider params off
const LOCAL_KEY_SETTINGS: CacheKeySettings = CacheKeySettings {
    cache_type: LiteLlmCacheType::Local,
    mode: CacheMode::DefaultOn,
    namespace: None,
    semantic_cache_scope: SemanticCacheScope::Key,
    include_provider_params: false,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheCreateOptions {
    pub tools: Option<Vec<Json>>,
    pub tool_choice: Option<Json>,
}

pub struct CacheCreateRequest<'request, F: RemoteMediaFetcher + Sync> {
    pub provider: CacheProvider,
    pub model: &'request str,
    pub messages: &'request [Message],
    pub api_base: Option<&'request str>,
    pub project: Option<&'request str>,
    pub location: Option<&'request str>,
    pub auth_token: Option<&'request SecretString>,
    pub gemini_api_key: Option<&'request SecretString>,
    pub extra_headers: &'request BTreeMap<String, String>,
    pub deadline: Duration,
    pub cached_content: Option<&'request str>,
    pub disable_transform: bool,
    pub now: Timestamp,
    pub minimum_token_override: Option<u64>,
    pub counter: &'request Counter,
    pub catalog: &'request ModelCatalog,
    pub capabilities: &'request (dyn ModelCapabilityLookup + Sync),
    pub fetcher: &'request F,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheCreateOutcome {
    pub messages: Vec<Message>,
    pub cached_content: Option<String>,
}

pub async fn check_and_create_cache<T, F, L, C>(
    transport: &T,
    request: &CacheCreateRequest<'_, F>,
    options: &mut CacheCreateOptions,
    hooks: &VerboseHooks<L, C>,
) -> Result<CacheCreateOutcome, CacheCreateError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    if let Some(name) = request.cached_content {
        return Ok(CacheCreateOutcome {
            messages: request.messages.to_vec(),
            cached_content: Some(name.to_owned()),
        });
    }
    let (cached, non_cached) =
        separate_cached_messages(request.messages, request.disable_transform);
    if cached.is_empty()
        || !ends_on_supported_turn(&cached)
        || !is_cacheable(request, options, &cached).await
    {
        return Ok(CacheCreateOutcome {
            messages: request.messages.to_vec(),
            cached_content: None,
        });
    }
    let cached_content = create_or_find(transport, request, options, &cached, hooks).await?;
    Ok(CacheCreateOutcome {
        messages: non_cached,
        cached_content,
    })
}

async fn is_cacheable<F>(
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

async fn create_or_find<T, F, L, C>(
    transport: &T,
    request: &CacheCreateRequest<'_, F>,
    options: &mut CacheCreateOptions,
    cached: &[Message],
    hooks: &VerboseHooks<L, C>,
) -> Result<Option<String>, CacheCreateError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let tools = options.tools.take();
    let tool_choice = options.tool_choice.take();
    let target = context_target(request)?;
    let cache_key = context_cache_key(
        cached,
        tools.as_deref(),
        tool_choice.as_ref(),
        request.model,
        hooks,
    )?;
    if let Some(name) = check_cache(
        transport,
        &target,
        &cache_key,
        request.extra_headers,
        request.deadline,
    )
    .await
    .map_err(|source| CacheCreateError::Endpoint { source })?
    {
        return Ok(Some(name));
    }
    let body = CreateCacheBody {
        cached: transform(request, cached, &cache_key).await?,
        tools: tools.as_deref(),
        tool_choice: tool_choice.as_ref(),
    };
    let encoded =
        serde_json::to_vec(&body).map_err(|source| CacheCreateError::Encode { source })?;
    let post = cache_create_request(&target, request.extra_headers, encoded, request.deadline)?;
    let reply = transport
        .send(post)
        .await
        .map_err(CacheCreateError::from_transport)?;
    created_cache(&reply).map(|created| created.name)
}

async fn transform<F>(
    request: &CacheCreateRequest<'_, F>,
    cached: &[Message],
    cache_key: &str,
) -> Result<CachedContentRequestBody, CacheCreateError>
where
    F: RemoteMediaFetcher + Sync,
{
    cached_content_request(
        &CachedContentRequest {
            model: request.model,
            messages: cached,
            provider: capability_provider(request.provider),
            cache_key,
            // the target builder already refused a Vertex request without both; Studio leaves them unused
            project: request.project.unwrap_or_default(),
            location: request.location.unwrap_or_default(),
            disable_transform: request.disable_transform,
            now: request.now,
        },
        request.capabilities,
        request.fetcher,
    )
    .await
    .map_err(|source| CacheCreateError::History { source })
}

fn context_target<F>(
    request: &CacheCreateRequest<'_, F>,
) -> Result<EndpointTarget, CacheCreateError>
where
    F: RemoteMediaFetcher + Sync,
{
    cache_target(&CacheTargetInput {
        provider: request.provider,
        api_base: request.api_base,
        project: request.project,
        location: request.location,
        auth_token: request.auth_token,
        gemini_api_key: request.gemini_api_key,
        model: Some(request.model),
    })
    .map_err(|source| CacheCreateError::Endpoint { source })
}

fn context_cache_key<L, C>(
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

fn created_cache(reply: &UpstreamResponse) -> Result<CreatedCache, CacheCreateError> {
    if reply.body.len() > CREATED_CACHE_LIMIT {
        return Err(CacheCreateError::TooLarge {
            limit: CREATED_CACHE_LIMIT,
        });
    }
    if !(200..300).contains(&reply.status) {
        return Err(CacheCreateError::Http {
            status: reply.status,
            message: str::from_utf8(&reply.body).map_or_else(
                |_| format!("{} bytes that are not UTF-8", reply.body.len()),
                str::to_owned,
            ),
        });
    }
    decode_created_cache(&reply.body)
}

const fn provider_name(provider: CacheProvider) -> &'static str {
    match provider {
        CacheProvider::Vertex => "vertex_ai",
        CacheProvider::VertexBeta => "vertex_ai_beta",
        CacheProvider::Gemini => "gemini",
    }
}

const fn capability_provider(provider: CacheProvider) -> VertexCapabilityProvider {
    match provider {
        CacheProvider::Vertex => VertexCapabilityProvider::VertexAi,
        CacheProvider::VertexBeta => VertexCapabilityProvider::VertexAiBeta,
        CacheProvider::Gemini => VertexCapabilityProvider::Gemini,
    }
}
