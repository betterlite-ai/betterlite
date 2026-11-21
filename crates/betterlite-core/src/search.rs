use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{SearchProvider, SearchQuery};
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;
use secrecy::SecretString;
use tracing::debug;

use crate::api::search_http::{SearchHttpCall, execute_search};
use crate::api::vector_store_http_write::service::LogContext;

mod call_log;
pub mod error;
mod params;
pub mod query;

use call_log::{SearchLog, log_search, record_provider_usage};
use error::{SearchCallError, SearchFailure, failure};
use params::{call_kwargs, merged_params, provider_params};
use query::search_query;

// ProviderConfigManager.get_provider_search_config: an executable config per call, or None for an unmapped name
pub trait SearchConfigLookup: Send + Sync {
    type Config: SearchProvider + Send + Sync;

    fn search_config(&self, provider: &str) -> Option<Self::Config>;
}

pub type HookError<Lookup> = <<Lookup as SearchConfigLookup>::Config as SearchProvider>::Error;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub max_results: Option<i64>,
    pub search_domain_filter: Option<Vec<String>>,
    pub max_tokens_per_page: Option<i64>,
    pub country: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: Json,
    pub search_provider: String,
    pub options: SearchOptions,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Duration>,
    // LiteLLM passes `extra_headers or {}`, so an absent map and an empty one are the same call
    pub extra_headers: Headers,
    pub kwargs: IndexMap<String, Json>,
}

pub struct SearchServices<'deps, Lookup> {
    pub lookup: &'deps Lookup,
    pub pool: &'deps ClientPool,
    pub tls: &'deps TlsPolicy,
    pub proxy: &'deps Arc<ProxyPolicy>,
    pub core: &'deps CoreSettings,
    pub network_mock: bool,
}

// search() and asearch() at litellm/search/main.py: one operation on the caller's task, so the logging state it writes is the caller's own
pub async fn asearch<Lookup>(
    request: &SearchRequest,
    services: &SearchServices<'_, Lookup>,
    log: &mut LogContext<'_, '_>,
) -> Result<SearchResponse, SearchFailure<HookError<Lookup>>>
where
    Lookup: SearchConfigLookup,
    HookError<Lookup>: 'static,
{
    let redactor = log.env.redactor;
    run(request, services, log)
        .await
        .map_err(|error| failure(error, &request.search_provider, redactor))
}

struct PreparedCall<Config> {
    config: Config,
    query: SearchQuery,
    params: Object,
    headers: Headers,
    complete_url: String,
}

async fn run<Lookup>(
    request: &SearchRequest,
    services: &SearchServices<'_, Lookup>,
    log: &mut LogContext<'_, '_>,
) -> Result<SearchResponse, SearchCallError<HookError<Lookup>>>
where
    Lookup: SearchConfigLookup,
    HookError<Lookup>: 'static,
{
    let prepared = prepare(request, services.lookup, log.state)?;
    let call = SearchHttpCall {
        query: &prepared.query,
        optional_params: &prepared.params,
        timeout: resolved_timeout(request.timeout, services.core)?,
        api_key: request.api_key.as_ref(),
        api_base: Some(prepared.complete_url),
        headers: prepared.headers,
        client: None,
        tls: services.tls,
        proxy: services.proxy,
    };
    let response = execute_search(
        call,
        &prepared.config,
        services.pool,
        log,
        services.network_mock,
    )
    .await?;
    record_provider_usage(log.state, &response).map_err(SearchCallError::Encode)?;
    Ok(response)
}

fn prepare<Lookup>(
    request: &SearchRequest,
    lookup: &Lookup,
    state: &mut LoggingState,
) -> Result<PreparedCall<Lookup::Config>, SearchCallError<HookError<Lookup>>>
where
    Lookup: SearchConfigLookup,
    HookError<Lookup>: 'static,
{
    let query = search_query(&request.query)?;
    let provider = &request.search_provider;
    let config = lookup
        .search_config(provider)
        .ok_or_else(|| SearchCallError::Unsupported {
            provider: provider.clone(),
        })?;
    debug!(provider = %provider, "Search call");
    let kwargs = call_kwargs(&request.kwargs);
    let merged = merged_params(&request.options, &kwargs);
    let params = provider_params(&merged).map_err(SearchCallError::Encode)?;
    let api_key = request.api_key.as_ref();
    let api_base = request.api_base.as_deref();
    let headers = config.validate_environment(request.extra_headers.clone(), api_key, api_base);
    let complete_url = config
        .complete_url(api_base, &params, None, api_key)
        .map_err(SearchCallError::Url)?;
    log_search(
        state,
        SearchLog {
            kwargs: &kwargs,
            optional_params: merged,
            complete_url: &complete_url,
            provider,
        },
    )?;
    Ok(PreparedCall {
        config,
        query,
        params,
        headers,
        complete_url,
    })
}

// `timeout or request_timeout`: an absent or zero timeout takes the configured request timeout
fn resolved_timeout<Hook>(
    timeout: Option<Duration>,
    core: &CoreSettings,
) -> Result<Duration, SearchCallError<Hook>>
where
    Hook: Error + 'static,
{
    match timeout {
        Some(value) if !value.is_zero() => Ok(value),
        Some(_) | None => {
            Duration::try_from_secs_f64(core.request_timeout_seconds).map_err(|source| {
                SearchCallError::Timeout {
                    seconds: core.request_timeout_seconds,
                    source,
                }
            })
        }
    }
}

#[path = "search/tests/async_execution.rs"]
mod async_execution;
#[path = "search/tests/environment_validation.rs"]
mod environment_validation;
#[path = "search/tests/exception_handling.rs"]
mod exception_handling;
#[path = "search/tests/http_dispatch.rs"]
mod http_dispatch;
#[path = "search/tests/kwargs_precedence.rs"]
mod kwargs_precedence;
#[path = "search/tests/logging.rs"]
mod logging;
#[path = "search/tests/optional_params.rs"]
mod optional_params;
#[path = "search/tests/provider_lookup.rs"]
mod provider_lookup;
#[path = "search/tests/query_validation.rs"]
mod query_validation;
#[path = "search/tests/support.rs"]
mod support;
#[path = "search/tests/sync_return.rs"]
mod sync_return;
#[path = "search/tests/url_resolution.rs"]
mod url_resolution;
