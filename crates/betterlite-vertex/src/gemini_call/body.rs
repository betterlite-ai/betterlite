use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::cache_create::orchestration::CacheCreateRequest;
use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_request::orchestration::CallRequest;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::probe::AccessToken;
use crate::vertex_embedding_call::post::HANDLER_DEFAULT_TIMEOUT;

use super::{CallDeps, GeminiCall};

// LiteLLM's transform arguments: the cache request carries the token and project resolved before the body
pub(super) fn body_request<'request, Cache, Fetch, Log, Console>(
    call: &'request GeminiCall<'request>,
    optional_params: &'request Object,
    deps: &'request CallDeps<'request, Cache, Fetch, Log, Console>,
    access: &'request AccessToken,
) -> CallRequest<'request, Fetch>
where
    Cache: CacheTransport,
    Fetch: RemoteMediaFetcher + Sync,
    Log: LoggerSink + Sync,
    Console: ConsoleSink + Sync,
{
    CallRequest {
        cache: CacheCreateRequest {
            provider: cache_provider(call.provider),
            model: call.model,
            messages: call.messages,
            api_base: call.api_base,
            project: Some(access.project.as_str()),
            location: call.location,
            auth_token: Some(&access.token),
            gemini_api_key: call.gemini_api_key,
            extra_headers: call.extra_headers,
            // LiteLLM gives the cache client the call timeout, and its own default when none was passed
            deadline: call.timeout.unwrap_or(HANDLER_DEFAULT_TIMEOUT.total),
            cached_content: None,
            disable_transform: false,
            now: call.now,
            // LiteLLM's minimum-token override is proxy-wide config, read outside this entry point
            minimum_token_override: None,
            counter: deps.counter,
            catalog: deps.catalog,
            capabilities: deps.capabilities,
            fetcher: deps.fetcher,
        },
        optional_params,
        litellm_params: call.litellm_params,
        modify_params: call.modify_params,
        schema_template: deps.schema_template,
        capabilities: deps.capabilities,
    }
}

const fn cache_provider(provider: VertexCapabilityProvider) -> CacheProvider {
    match provider {
        VertexCapabilityProvider::VertexAi => CacheProvider::Vertex,
        VertexCapabilityProvider::VertexAiBeta => CacheProvider::VertexBeta,
        VertexCapabilityProvider::Gemini => CacheProvider::Gemini,
    }
}
