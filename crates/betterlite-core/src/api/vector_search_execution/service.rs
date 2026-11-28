use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use secrecy::ExposeSecret as _;
use url::Url;

use self::logging::log_http;
use self::request::{select, transform, validated_headers};
use self::signing::{SignInput, SigningSeam, sign};
use super::config::{
    Params, QueryEmbeddingExecutor, ResponseHeaders, SearchConfig, SearchQuery, VectorStoreLogging,
    VectorStoreResponse,
};
use super::error::{SearchError, TIMEOUT_MESSAGE, convert_error, reply_facts, transport_facts};
use super::params::ssl_verify;
use crate::api::vector_store_http_write::error::{HeaderMap, header_text};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest, WriteTransport,
};

pub mod direct;
mod logging;
mod request;
pub mod signing;

pub struct SearchDeps<'deps, S: ClientSource> {
    pub config: Arc<dyn SearchConfig>,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct SearchCall<'call> {
    pub vector_store_id: &'call str,
    pub query: &'call SearchQuery,
    pub optional_params: &'call Params,
    pub litellm_params: &'call Params,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call Params>,
    pub timeout: Option<Duration>,
    pub embedding_executor: Option<&'call dyn QueryEmbeddingExecutor>,
}

// The selected httpx client class, or the one the caller supplied for this mode
pub(super) enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    pub(super) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

pub struct PreparedSearch<'deps, Client> {
    config: Arc<dyn SearchConfig>,
    client: Selected<'deps, Client>,
    request: WriteRequest,
    logging: VectorStoreLogging,
}

// The HTTP handler's front half: client, validated headers, base URL, transform, signing and the pre-call
pub async fn prepare<'deps, S, G>(
    deps: &SearchDeps<'deps, S>,
    seam: &G,
    mode: ClientKind,
    call: &SearchCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedSearch<'deps, S::Client>, SearchError>
where
    S: ClientSource<Client: WriteTransport>,
    G: SigningSeam + ?Sized,
{
    let client = select(deps, mode, ssl_verify(call.litellm_params)?)?;
    let headers = validated_headers(deps.config.as_ref(), call)?;
    let api_base = deps.config.complete_url(call.litellm_params)?;
    let mut logging = VectorStoreLogging::default();
    let (url, body) = transform(deps.config.as_ref(), mode, call, &api_base, &mut logging).await?;
    merge_details(log.state, &logging);
    let input = SignInput {
        headers,
        optional_params: merged_params(call.litellm_params, call.optional_params),
        request_data: body.clone(),
        api_base: url.clone(),
    };
    let (signed_headers, signed) = sign(Arc::clone(&deps.config), seam, mode, input).await?;
    log_http(&api_base, &signed_headers, &body, log)?;
    let request = WriteRequest {
        method: Method::POST,
        url: Url::parse(&url).map_err(|source| SearchError::RequestUrl { url, source })?,
        headers: wire_headers(&signed_headers)?,
        body: wire_body(&body, signed.as_deref())?,
        timeout: call.timeout,
    };
    Ok(PreparedSearch {
        config: Arc::clone(&deps.config),
        client,
        request,
        logging: VectorStoreLogging {
            model_call_details: logged_details(log.state),
        },
    })
}

// LiteLLM hands the transform and the pre-call one logging object, so what a transform records reaches the pre-call's metadata
fn merge_details(state: &mut LoggingState, logging: &VectorStoreLogging) {
    state
        .model_call_details
        .extend(logging.model_call_details.clone());
}

// After the pre-call the same object carries the call metadata, which the response transform reads
fn logged_details(state: &LoggingState) -> Params {
    state
        .model_call_details
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// One send, no retry: a timeout takes the handler's own 408, and any other failure goes through _handle_error
pub async fn send<Client>(
    prepared: PreparedSearch<'_, Client>,
) -> Result<VectorStoreSearchResponse, SearchError>
where
    Client: WriteTransport,
{
    let PreparedSearch {
        config,
        client,
        request,
        mut logging,
    } = prepared;
    let reply = client.get().send(request).await;
    let answer = match reply {
        Ok(answer) => answer,
        Err(TransportError::ConnectTimeout(_) | TransportError::Deadline(_)) => {
            return Err(SearchError::Provider(config.error_class(
                TIMEOUT_MESSAGE,
                408,
                HeaderMap::new(),
            )));
        }
        Err(error) => {
            return Err(SearchError::Provider(convert_error(
                &transport_facts(&error),
                config.as_ref(),
            )));
        }
    };
    if answer.status < 400 {
        return config.transform_search_response(&response_of(&answer), &mut logging);
    }
    Err(SearchError::Provider(convert_error(
        &reply_facts(&answer),
        config.as_ref(),
    )))
}

// dict(litellm_params) is updated with the search optional params, so a search key wins over the same litellm key
fn merged_params(litellm_params: &Params, optional_params: &Params) -> Params {
    let mut merged = litellm_params.clone();
    merged.extend(optional_params.clone());
    merged
}

fn wire_headers(headers: &OpenAiHeaders) -> Result<HttpHeaders, SearchError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| SearchError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value = HeaderValue::from_str(value.expose_secret()).map_err(|source| {
            SearchError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// A signed body, empty or not, is sent as it is; only a null signed body is serialized, exactly once
fn wire_body(body: &Params, signed: Option<&[u8]>) -> Result<RequestBody, SearchError> {
    signed.map_or_else(
        || {
            serde_json::to_vec(body)
                .map(RequestBody::Json)
                .map_err(SearchError::Encode)
        },
        |bytes| Ok(RequestBody::Json(bytes.to_vec())),
    )
}

fn response_of(reply: &UpstreamResponse) -> VectorStoreResponse {
    let headers: ResponseHeaders = reply
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect();
    VectorStoreResponse {
        status_code: reply.status,
        headers,
        body: reply.body.clone(),
    }
}
