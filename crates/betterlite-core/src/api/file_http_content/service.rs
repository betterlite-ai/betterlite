use std::future::Future;
use std::num::NonZeroUsize;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_pre_post::pre_call::pre_call;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::{
    FileContentStreamingResult, flatten_headers, read_error_body,
};
use http::header::{CONTENT_ENCODING, CONTENT_LENGTH, HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use url::Url;

use crate::api::vector_store_http_write::error::ClientError;

use super::config::{
    BodyStream, ContentParams, FileContentConfig, Headers, QueryParams, StreamContext,
};
use super::error::ContentError;

mod facts;
mod proxied;
mod query;

pub struct ContentRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HttpHeaders,
    pub timeout: Option<Duration>,
}

pub struct BufferedReply {
    pub status: u16,
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: Vec<u8>,
}

pub struct StreamedReply {
    pub status: u16,
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: BodyStream,
}

// The selected client: both buffered modes run over one pooled client class, and streaming asks it for the body unread
pub trait ContentTransport: Send + Sync {
    fn send(
        &self,
        request: ContentRequest,
    ) -> impl Future<Output = Result<BufferedReply, TransportError>> + Send;

    fn send_stream(
        &self,
        request: ContentRequest,
    ) -> impl Future<Output = Result<StreamedReply, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client: ContentTransport;

    fn client(&self) -> Result<Self::Client, ClientError>;
}

pub struct ContentDeps<'deps, C: FileContentConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

// The three handlers' inputs: the request, the litellm_params it reads, the caller's headers and the caller's timeout
pub struct RetrieveCall<'call> {
    pub request: &'call FileContentRequest,
    pub params: &'call ContentParams,
    pub headers: &'call Headers,
    pub timeout: Option<Duration>,
}

pub struct PreparedContent<'deps, C: FileContentConfig, S: ClientSource> {
    config: &'deps C,
    client: S::Client,
    url: Url,
    query: QueryParams,
    headers: HttpHeaders,
    params: ContentParams,
    timeout: Option<Duration>,
}

// _prepare_file_content_request: the transform's URL and query, the validated environment, the pre-call, then the wire headers
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow ContentDeps<'deps, C, S>,
    call: &RetrieveCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedContent<'borrow, C, S>, ContentError>
where
    'deps: 'borrow,
    C: FileContentConfig,
    S: ClientSource,
{
    let client = deps.source.client()?;
    let (target, query) = deps
        .config
        .transform_content_request(call.request, call.params)?;
    let url = match Url::parse(&target) {
        Ok(parsed) => parsed,
        Err(source) => {
            return Err(ContentError::RequestUrl {
                url: target,
                source,
            });
        }
    };
    let headers = deps
        .config
        .validate_environment(call.headers.clone(), call.params)?;
    log_pre_call(&url, &headers, &call.request.file_id, log)?;
    let wire = wire_headers(&headers)?;
    Ok(PreparedContent {
        config: deps.config,
        client,
        url,
        query,
        headers: wire,
        params: call.params.clone(),
        timeout: call.timeout,
    })
}

// LiteLLM's pre_call(input="", api_key="") over the transformed URL, the validated headers and the file id
fn log_pre_call(
    api_base: &Url,
    headers: &Headers,
    file_id: &str,
    log: &mut LogContext<'_, '_>,
) -> Result<(), ContentError> {
    let recorded = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    let mut passthrough = IndexMap::new();
    passthrough.insert(String::from("file_id"), Json::Text(file_id.to_owned()));
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.as_str().to_owned()),
            headers: Some(recorded),
            complete_input_dict: None,
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    };
    pre_call(log.state, &record, log.env)?;
    Ok(())
}

fn wire_headers(headers: &Headers) -> Result<HttpHeaders, ContentError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| ContentError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| ContentError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// retrieve_file_content and async_retrieve_file_content share this GET: no per-call timeout, and a status of 400 or more raises the provider's error
pub async fn retrieve_file_content<C, S>(
    prepared: PreparedContent<'_, C, S>,
    logging: &mut LoggingState,
) -> Result<BinaryContent, ContentError>
where
    C: FileContentConfig,
    S: ClientSource,
{
    let PreparedContent {
        config,
        client,
        url,
        query,
        headers,
        params,
        ..
    } = prepared;
    let pairs: Vec<(String, String)> = query.into_iter().collect();
    let reply = client
        .send(ContentRequest {
            method: Method::GET,
            url,
            query: pairs,
            headers,
            timeout: None,
        })
        .await
        .map_err(|error| ContentError::Provider(facts::transport_error(config, &error)))?;
    if reply.status >= 400 {
        return Err(ContentError::Provider(facts::status_error(
            config,
            reply.status,
            &reply.headers,
            &reply.body,
        )));
    }
    config.transform_content_response(&facts::buffered_response(reply), logging, &params)
}

// async_retrieve_file_content_streaming: the prepared query overrides the transform's own, and the body stays unread until the caller iterates
pub async fn retrieve_content_streaming<C, S>(
    prepared: PreparedContent<'_, C, S>,
    chunk_size: NonZeroUsize,
    logging: &mut LoggingState,
) -> Result<FileContentStreamingResult, ContentError>
where
    C: FileContentConfig,
    S: ClientSource,
{
    let PreparedContent {
        config,
        client,
        url,
        query,
        headers,
        params,
        timeout,
    } = prepared;
    let target = query::streaming_url(&url, &query);
    let reply = client
        .send_stream(ContentRequest {
            method: Method::GET,
            url: target,
            query: Vec::new(),
            headers,
            timeout,
        })
        .await
        .map_err(|error| ContentError::Provider(facts::transport_error(config, &error)))?;
    let StreamedReply {
        status,
        url: answered,
        headers: response_headers,
        body,
    } = reply;
    if status >= 400 {
        let error_body = read_error_body(body).await;
        return Err(ContentError::Provider(facts::status_error(
            config,
            status,
            &response_headers,
            &error_body,
        )));
    }
    let decoded = decoded_body_headers(&response_headers);
    let context = StreamContext {
        request_url: answered.as_str(),
        logging,
        params: &params,
    };
    Ok(config.transform_content_stream(body, &decoded, chunk_size, context))
}

// aiter_bytes yields the decoded body, so a content-encoding other than identity leaves the transfer headers describing the wire bytes
#[must_use]
pub fn decoded_body_headers(headers: &HttpHeaders) -> HttpHeaders {
    let flattened = flatten_headers(headers);
    let identity = match flattened.get("content-encoding") {
        None => true,
        Some(value) => value.eq_ignore_ascii_case("identity"),
    };
    if identity {
        return headers.clone();
    }
    let mut kept = HttpHeaders::new();
    for (name, value) in headers {
        if *name != CONTENT_ENCODING && *name != CONTENT_LENGTH {
            kept.append(name.clone(), value.clone());
        }
    }
    kept
}
