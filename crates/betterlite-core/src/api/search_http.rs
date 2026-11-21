use std::error::Error;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    OutboundSearch, RequestInput, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::api::vector_store_http_write::error::display_facts;
use crate::api::vector_store_http_write::service::LogContext;

mod tests;
mod transport;

use transport::{
    Timing, conversion, exchange, failure_facts, finish, outbound_request, select_client,
};

pub struct SearchHttpCall<'call> {
    pub query: &'call SearchQuery,
    pub optional_params: &'call Object,
    pub timeout: Duration,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<String>,
    pub headers: Headers,
    pub client: Option<ProxiedClient>,
    pub tls: &'call TlsPolicy,
    pub proxy: &'call Arc<ProxyPolicy>,
}

#[derive(Debug, Error)]
pub enum SearchHttpError<HookError>
where
    HookError: Error + 'static,
{
    #[error("{}", .provider.message)]
    Provider { provider: Box<ProviderError> },
    #[error("{0}")]
    Hook(#[source] HookError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
}

pub(super) struct PreparedSearch<Body> {
    validated: Headers,
    body: Body,
    complete_url: String,
    signed_headers: Headers,
    signed_body: Option<Vec<u8>>,
}

// The Sync bound keeps the returned future Send, because the provider is borrowed across the send
pub async fn execute_search<Provider>(
    call: SearchHttpCall<'_>,
    provider: &Provider,
    pool: &ClientPool,
    log: &mut LogContext<'_, '_>,
    network_mock: bool,
) -> Result<SearchResponse, SearchHttpError<Provider::Error>>
where
    Provider: SearchProvider + Sync,
    Provider::Error: 'static,
{
    let prepared = prepare_search(&call, provider)?;
    let encoded = serde_json::to_vec(&prepared.body)
        .map_err(|source| conversion(provider, &display_facts(source.to_string())))?;
    log_pre_call(&call, &prepared, &encoded, log)?;
    let timeout = call.timeout;
    let client = select_client(pool, call, network_mock)
        .map_err(|error| conversion(provider, &display_facts(error.to_string())))?;
    let timing = Timing::new(provider.http_method(), timeout, client.limits.read_timeout);
    let outbound = outbound_request(prepared, &encoded, &timing)
        .map_err(|caught| conversion(provider, &failure_facts(&caught, &timing)))?;
    let reply = exchange(&client, outbound)
        .await
        .map_err(|caught| conversion(provider, &failure_facts(&caught, &timing)))?;
    finish(provider, &reply)
}

fn prepare_search<Provider>(
    call: &SearchHttpCall<'_>,
    provider: &Provider,
) -> Result<PreparedSearch<Provider::Body>, SearchHttpError<Provider::Error>>
where
    Provider: SearchProvider,
    Provider::Error: 'static,
{
    let api_base = call.api_base.as_deref();
    let validated = provider.validate_environment(call.headers.clone(), call.api_key, api_base);
    let body = provider
        .transform_search_request(&RequestInput {
            query: call.query,
            optional_params: call.optional_params,
            api_key: call.api_key,
            api_base,
            headers: &validated,
        })
        .map_err(SearchHttpError::Hook)?;
    let complete_url = provider
        .complete_url(api_base, call.optional_params, Some(&body), call.api_key)
        .map_err(SearchHttpError::Hook)?;
    let outbound = OutboundSearch {
        api_base: &complete_url,
        api_key: call.api_key,
        optional_params: call.optional_params,
        body: &body,
    };
    let (signed_headers, signed_body) = provider
        .sign_request(validated.clone(), &outbound)
        .map_err(SearchHttpError::Hook)?;
    Ok(PreparedSearch {
        validated,
        body,
        complete_url,
        signed_headers,
        signed_body,
    })
}

fn log_pre_call<Body>(
    call: &SearchHttpCall<'_>,
    prepared: &PreparedSearch<Body>,
    encoded: &[u8],
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    let mut logged = IndexMap::new();
    for (name, value) in &prepared.validated {
        logged.insert(name.clone(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: query_text(call.query),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(prepared.complete_url.clone()),
            headers: Some(logged),
            complete_input_dict: Some(
                serde_json::from_slice(encoded)
                    .unwrap_or_else(|_| RawBody::Text(body_text(encoded))),
            ),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)
}

fn query_text(query: &SearchQuery) -> Json {
    match query {
        SearchQuery::Single(text) => Json::Text(text.clone()),
        SearchQuery::Multiple(items) => {
            Json::Text(serde_json::to_string(items).unwrap_or_default())
        }
    }
}

fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}
