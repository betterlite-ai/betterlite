use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::{ClientChoice, ClientPool, PoolError};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::search::provider::{HttpMethod, ProviderResponse, SearchProvider};
use betterlite_openai::search::records::SearchResponse;
use http::HeaderMap;
use http::Method;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::{ParseError, Url};

use crate::api::vector_store_http_write::error::{
    FailureFacts, display_facts, handle_error, header_text, timeout_facts,
};
use crate::api::vector_store_http_write::service::ClientKind;

use super::{PreparedSearch, SearchHttpCall, SearchHttpError, body_text};

#[derive(Debug, Error)]
pub(super) enum ExchangeFailure {
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[derive(Debug, Error)]
pub(super) enum RequestError {
    #[error("the search request URL {url} is not a URL")]
    Url {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
}

pub(super) struct Timing {
    method: HttpMethod,
    deadline: Duration,
    started: Instant,
}

impl Timing {
    pub(super) fn new(method: HttpMethod, timeout: Duration, read_timeout: Duration) -> Self {
        Self {
            method,
            deadline: deadline(method, timeout, read_timeout),
            started: Instant::now(),
        }
    }
}

// GET sends no per-request timeout, so the client's own read timeout is its deadline
const fn deadline(method: HttpMethod, timeout: Duration, read_timeout: Duration) -> Duration {
    match method {
        HttpMethod::Get => read_timeout,
        HttpMethod::Post => timeout,
    }
}

pub(super) fn select_client(
    pool: &ClientPool,
    call: SearchHttpCall<'_>,
    network_mock: bool,
) -> Result<ProxiedClient, PoolError> {
    match pool.select(call.client, network_mock, call.tls, call.proxy)? {
        ClientChoice::Supplied(client) | ClientChoice::Pooled(client) => Ok(client),
        ClientChoice::Mock => {
            let mut mocked = pool.client(call.tls, call.proxy)?;
            mocked.network_mock = true;
            Ok(mocked)
        }
    }
}

pub(super) fn outbound_request<Body>(
    prepared: PreparedSearch<Body>,
    encoded: &[u8],
    timing: &Timing,
) -> Result<OutboundRequest, ExchangeFailure> {
    let text = prepared.complete_url;
    let url = Url::parse(&text).map_err(|source| RequestError::Url { url: text, source })?;
    Ok(OutboundRequest {
        method: wire_method(timing.method),
        url,
        query: Vec::new(),
        headers: wire_headers(&prepared.signed_headers)?,
        body: wire_body(prepared.signed_body.as_deref(), encoded, timing.method),
        deadline: timing.deadline,
    })
}

const fn wire_method(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
    }
}

fn wire_headers(headers: &Headers) -> Result<HeaderMap, RequestError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| RequestError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| RequestError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn wire_body(signed: Option<&[u8]>, encoded: &[u8], method: HttpMethod) -> RequestBody {
    match (method, signed) {
        (HttpMethod::Get, Some(_) | None) => RequestBody::Empty,
        (HttpMethod::Post, Some(data)) => RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/json"),
            data: data.to_vec(),
        },
        (HttpMethod::Post, None) => RequestBody::Json(encoded.to_vec()),
    }
}

pub(super) async fn exchange(
    client: &ProxiedClient,
    request: OutboundRequest,
) -> Result<UpstreamResponse, ExchangeFailure> {
    client
        .send_following(request)
        .await?
        .buffered(client.limits.body_bytes)
        .await
        .map_err(ExchangeFailure::from)
}

pub(super) fn search_failure<Provider>(
    provider: &Provider,
    failure: &ExchangeFailure,
    timing: &Timing,
) -> SearchHttpError<Provider::Error>
where
    Provider: SearchProvider,
    Provider::Error: 'static,
{
    let timed_out = matches!(
        failure,
        ExchangeFailure::Transport(TransportError::Deadline(_))
    );
    let facts = if timed_out && timing.method == HttpMethod::Post {
        timeout_facts(
            ClientKind::Asynchronous,
            Some(timing.deadline),
            timing.started.elapsed(),
        )
    } else {
        display_facts(failure.to_string())
    };
    conversion(provider, &facts)
}

// A failure with no HTTP answer behind it keeps the converter's synthesized 500 and its own text
pub(super) fn text_failure<Provider>(
    provider: &Provider,
    display: String,
) -> SearchHttpError<Provider::Error>
where
    Provider: SearchProvider,
    Provider::Error: 'static,
{
    conversion(provider, &display_facts(display))
}

pub(super) fn conversion<Provider>(
    provider: &Provider,
    facts: &FailureFacts,
) -> SearchHttpError<Provider::Error>
where
    Provider: SearchProvider,
    Provider::Error: 'static,
{
    let converted = handle_error(facts, None);
    let mut error =
        provider.error_class(&converted.message, converted.status_code, &Headers::new());
    error.status_code_is_synthesized = converted.status_code_is_synthesized;
    SearchHttpError::Provider {
        provider: Box::new(error),
    }
}

pub(super) fn finish<Provider>(
    provider: &Provider,
    reply: &UpstreamResponse,
) -> Result<SearchResponse, SearchHttpError<Provider::Error>>
where
    Provider: SearchProvider,
    Provider::Error: 'static,
{
    let text = body_text(&reply.body);
    if !(200..300).contains(&reply.status) {
        let headers = match provider.http_method() {
            HttpMethod::Get => text_headers(&reply.headers),
            HttpMethod::Post => masked_headers(reply),
        };
        let error = provider.http_error(&ProviderResponse {
            status_code: reply.status,
            headers: &headers,
            text: &text,
        });
        return Err(SearchHttpError::Provider {
            provider: Box::new(error),
        });
    }
    let headers = text_headers(&reply.headers);
    provider
        .transform_search_response(&ProviderResponse {
            status_code: reply.status,
            headers: &headers,
            text: &text,
        })
        .map_err(SearchHttpError::Hook)
}

fn text_headers(headers: &HeaderMap) -> Headers {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}

// The POST helper raises httpx's masked HTTPStatusError, whose headers drop content-encoding and recompute content-length
fn masked_headers(reply: &UpstreamResponse) -> Headers {
    let mut headers = text_headers(&reply.headers);
    headers.remove("content-encoding");
    headers.remove("content-length");
    if !reply.body.is_empty() {
        headers.insert(String::from("content-length"), reply.body.len().to_string());
    }
    headers
}
