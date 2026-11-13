use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::Counter;
use jiff::Timestamp;
use secrecy::SecretString;

use crate::cache_lookup::lookup::check_cache;
use crate::cache_lookup::target::{CacheProvider, CacheTargetInput, cache_target};
use crate::cache_lookup::transport::CacheTransport;
use crate::cache_prefix::selection::{ends_on_supported_turn, separate_cached_messages};
use crate::cache_prefix::transform::{
    CachedContentRequest, CachedContentRequestBody, cached_content_request,
};
use crate::endpoint_target::target::EndpointTarget;
use crate::model_policy::capabilities::{ModelCapabilityLookup, VertexCapabilityProvider};

use super::eligible::is_cacheable;
use super::error::CacheCreateError;
use super::key::context_cache_key;
use super::records::{CREATED_CACHE_LIMIT, CreatedCache, decode_created_cache};
use super::request::{CreateCacheBody, cache_create_request};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheCreateOptions {
    // the provider-mapped tools and tool config LiteLLM's optional params carry by the time context caching runs
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

// the endpoint, key and cached block an eligible request sends, with the options already stripped
struct CachePlan<'plan> {
    target: EndpointTarget,
    cache_key: String,
    cached: &'plan [Message],
    tools: Option<Vec<Json>>,
    tool_choice: Option<Json>,
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
    let plan = plan_cache(request, options, &cached, hooks)?;
    let found = check_cache(
        transport,
        &plan.target,
        &plan.cache_key,
        request.extra_headers,
        request.deadline,
    )
    .await
    .map_err(|source| CacheCreateError::Endpoint { source })?;
    // LiteLLM's `if google_cache_name:` treats an empty name as unusable, so a new cache is created instead
    if let Some(name) = found.filter(|name| !name.is_empty()) {
        return Ok(CacheCreateOutcome {
            messages: non_cached,
            cached_content: Some(name),
        });
    }
    create_or_find(transport, request, options, plan, non_cached).await
}

async fn create_or_find<T, F>(
    transport: &T,
    request: &CacheCreateRequest<'_, F>,
    options: &mut CacheCreateOptions,
    plan: CachePlan<'_>,
    non_cached: Vec<Message>,
) -> Result<CacheCreateOutcome, CacheCreateError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
{
    let created = post_create(transport, request, &plan).await?;
    // LiteLLM returns the uncached messages for an unusable name, which drops the prefix and the tools it removed; betterlite sends the request uncached and complete
    let Some(name) = created.name.filter(|name| !name.is_empty()) else {
        options.tools = plan.tools;
        options.tool_choice = plan.tool_choice;
        return Ok(CacheCreateOutcome {
            messages: request.messages.to_vec(),
            cached_content: None,
        });
    };
    Ok(CacheCreateOutcome {
        messages: non_cached,
        cached_content: Some(name),
    })
}

fn plan_cache<'plan, F, L, C>(
    request: &CacheCreateRequest<'_, F>,
    options: &mut CacheCreateOptions,
    cached: &'plan [Message],
    hooks: &VerboseHooks<L, C>,
) -> Result<CachePlan<'plan>, CacheCreateError>
where
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let tools = options.tools.take();
    let tool_choice = options.tool_choice.take();
    let target = cache_target(&CacheTargetInput {
        provider: request.provider,
        api_base: request.api_base,
        project: request.project,
        location: request.location,
        auth_token: request.auth_token,
        gemini_api_key: request.gemini_api_key,
        model: Some(request.model),
    })
    .map_err(|source| CacheCreateError::Endpoint { source })?;
    let cache_key = context_cache_key(
        cached,
        tools.as_deref(),
        tool_choice.as_ref(),
        request.model,
        hooks,
    )?;
    Ok(CachePlan {
        target,
        cache_key,
        cached,
        tools,
        tool_choice,
    })
}

async fn post_create<T, F>(
    transport: &T,
    request: &CacheCreateRequest<'_, F>,
    plan: &CachePlan<'_>,
) -> Result<CreatedCache, CacheCreateError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
{
    let body = CreateCacheBody {
        cached: transform(request, plan).await?,
        tools: plan.tools.as_deref(),
        tool_choice: plan.tool_choice.as_ref(),
    };
    let encoded =
        serde_json::to_vec(&body).map_err(|source| CacheCreateError::Encode { source })?;
    let post = cache_create_request(
        &plan.target,
        request.extra_headers,
        encoded,
        request.deadline,
    )?;
    let reply = transport
        .send(post)
        .await
        .map_err(CacheCreateError::from_transport)?;
    created_cache(&reply)
}

async fn transform<F>(
    request: &CacheCreateRequest<'_, F>,
    plan: &CachePlan<'_>,
) -> Result<CachedContentRequestBody, CacheCreateError>
where
    F: RemoteMediaFetcher + Sync,
{
    cached_content_request(
        &CachedContentRequest {
            model: request.model,
            messages: plan.cached,
            provider: capability_provider(request.provider),
            cache_key: &plan.cache_key,
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

const fn capability_provider(provider: CacheProvider) -> VertexCapabilityProvider {
    match provider {
        CacheProvider::Vertex => VertexCapabilityProvider::VertexAi,
        CacheProvider::VertexBeta => VertexCapabilityProvider::VertexAiBeta,
        CacheProvider::Gemini => VertexCapabilityProvider::Gemini,
    }
}
