use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use crate::api::vector_store_http_write::error::{handle_error, status_facts};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource, SuppliedClient};

use super::config::{QueryParams, ReadConfig, ReadParams};
use super::error::ReadError;

use self::request::{select_client, validated_headers};

mod facts;
mod proxied;
mod request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Retrieve,
    Content,
}

#[derive(Clone)]
pub struct ReadRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HttpHeaders,
    // LiteLLM's read handlers take a timeout and never hand it to the GET
    pub timeout: Option<Duration>,
}

pub trait ReadTransport: Send + Sync {
    fn send(
        &self,
        request: ReadRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub struct ReadDeps<'deps, C: ReadConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

#[derive(Debug, Clone, Copy)]
pub struct ReadTarget<'call> {
    pub vector_store_id: &'call str,
    pub file_id: &'call str,
    pub params: &'call ReadParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    // LiteLLM's read handlers accept the caller's timeout and never pass it on
    pub timeout: Option<Duration>,
}

pub struct ReadCall<'call, Client> {
    pub target: ReadTarget<'call>,
    pub client: Option<SuppliedClient<'call, Client>>,
    pub mode: ClientKind,
    pub operation: Operation,
}

enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

pub struct PreparedRead<'deps, C: ReadConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: ReadRequest,
    operation: Operation,
}

// The four handlers' shared front half: client selection, environment validation, the complete store-files URL, the provider transform and the pre-call
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow ReadDeps<'deps, C, S>,
    mode: ClientKind,
    operation: Operation,
    target: &ReadTarget<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRead<'borrow, C, S>, ReadError>
where
    'deps: 'borrow,
    C: ReadConfig,
    S: ClientSource,
{
    let client = select_client(deps, mode, target.params.ssl_verify)?;
    let headers = validated_headers(deps.config, target.extra_headers, target.params);
    let api_base = deps
        .config
        .complete_url(target.params, target.vector_store_id)?;
    let (transformed, query) =
        request_transform(deps.config, operation, target.file_id, &api_base)?;
    let url = match Url::parse(&transformed) {
        Ok(parsed) => parsed,
        Err(source) => {
            return Err(ReadError::RequestUrl {
                url: transformed,
                source,
            });
        }
    };
    log_pre_call(&api_base, &headers, &query, log)?;
    let wire = wire_headers(&headers)?;
    Ok(PreparedRead {
        config: deps.config,
        client,
        request: ReadRequest {
            method: Method::GET,
            url,
            query: query.into_iter().collect(),
            headers: wire,
            timeout: None,
        },
        operation,
    })
}

fn request_transform<C>(
    config: &C,
    operation: Operation,
    file_id: &str,
    api_base: &str,
) -> Result<(String, QueryParams), ReadError>
where
    C: ReadConfig,
{
    let transformed = match operation {
        Operation::Retrieve => config.transform_retrieve_request(file_id, api_base),
        Operation::Content => config.transform_content_request(file_id, api_base),
    };
    transformed.map_err(ReadError::Prepare)
}

// LiteLLM's pre_call(input="", api_key="") over the complete store-files URL, the validated headers and the transform's parameter map
fn log_pre_call(
    api_base: &str,
    headers: &OpenAiHeaders,
    params: &QueryParams,
    log: &mut LogContext<'_, '_>,
) -> Result<(), ReadError> {
    let mut recorded = IndexMap::new();
    for (name, value) in headers {
        recorded.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let complete: IndexMap<String, Json> = params
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(recorded),
            complete_input_dict: Some(RawBody::Object(complete)),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

fn wire_headers(headers: &OpenAiHeaders) -> Result<HttpHeaders, ReadError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| ReadError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value = HeaderValue::from_str(value.expose_secret()).map_err(|source| {
            ReadError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// One GET, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
pub async fn send<C, S>(prepared: PreparedRead<'_, C, S>) -> Result<Json, ReadError>
where
    C: ReadConfig,
    S: ClientSource<Client: ReadTransport>,
{
    let PreparedRead {
        config,
        client,
        request,
        operation,
    } = prepared;
    let reply = client
        .get()
        .send(request)
        .await
        .map_err(|error| ReadError::Provider(facts::transport_error(config, &error)))?;
    if reply.status >= 400 {
        let mapped = handle_error(&status_facts(&reply), None);
        return Err(ReadError::Provider(config.error_class(
            &mapped.message,
            mapped.status_code,
            mapped.headers,
        )));
    }
    let response = facts::http_response(&reply);
    let answer = match operation {
        Operation::Retrieve => config.transform_retrieve_response(&response),
        Operation::Content => config.transform_content_response(&response),
    };
    answer.map_err(ReadError::Provider)
}
