use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_http::proxy::{pool::PoolError, transport_error::TransportError};
use betterlite_logging::logging_pre_post::{args::RawBody, error::PrePostError};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::{config, error, records};
use betterlite_openai::containers::container_file_http::request::{self, SslVerify};
use betterlite_openai::containers::container_file_http::transport::{self, Selected};
use betterlite_openai::containers::container_file_http::{self as file, response};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use thiserror::Error;

// LiteLLM's declared timeout default for both container handlers
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

// HTTPHandler.post turns a timeout into litellm.Timeout with this status; _handle_error gives every other exception a synthesized 500
const REQUEST_TIMEOUT_STATUS: u16 = 408;
const SERVER_ERROR_STATUS: u16 = 500;
// B3363's container fetch carries no body and no deadline, so a collection call sends through its own request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRequest {
    pub method: transport::ContainerMethod,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub query: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
}

pub type CollectionReply = Result<transport::ContainerHttpReply, transport::TransferError>;

pub trait CollectionTransport: Send + Sync {
    fn send(&self, request: &CollectionRequest) -> impl Future<Output = CollectionReply> + Send;
}

// B173 owns the pool behind this: the blocking and async clients are different types, as HTTPHandler and AsyncHTTPHandler are
pub trait CollectionClientSource: Send + Sync {
    type Blocking: CollectionTransport;
    type Asynchronous: CollectionTransport;

    fn blocking_client(&self, ssl_verify: Option<&SslVerify>) -> Result<Self::Blocking, PoolError>;

    fn async_client(&self, ssl_verify: Option<&SslVerify>)
    -> Result<Self::Asynchronous, PoolError>;
}

pub struct ContainerCreateCall<'call> {
    pub name: &'call str,
    pub params: &'call Object,
    pub config: &'call dyn config::BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct ContainerListCall<'call> {
    pub config: &'call dyn config::BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub query: config::ContainerListQuery<'call>,
    // LiteLLM takes the timeout and never sends it, so a GET keeps the selected client's own deadline
    pub timeout: Option<Duration>,
}

#[derive(Debug, Error)]
pub enum ContainerCollectionError {
    #[error(transparent)]
    Provider(#[from] error::BaseLlmException),
    #[error(transparent)]
    Config(#[from] error::ContainerError),
    #[error(transparent)]
    Params(#[from] file::ContainerFileHttpError),
    #[error(transparent)]
    Client(#[from] PoolError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the container request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the blocking path was asked to wait for a client that never answered")]
    Suspended,
}

fn poll_once<Running: Future>(
    future: Running,
) -> Result<Running::Output, ContainerCollectionError> {
    let mut pinned = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match pinned.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => Ok(outcome),
        Poll::Pending => Err(ContainerCollectionError::Suspended),
    }
}

pub fn container_create_handler<Source: CollectionClientSource>(
    call: &ContainerCreateCall<'_>,
    source: &Source,
    supplied: Option<&Source::Blocking>,
    hook: &dyn file::PreCallHook,
) -> Result<records::ContainerObject, ContainerCollectionError> {
    let params = request::read_params(call.litellm_params)?;
    let ssl_verify = params.ssl_verify.as_ref();
    let client = Selected::choose(supplied, || source.blocking_client(ssl_verify))?;
    poll_once(create_body(client.get(), &params, call, hook))?
}

pub async fn acontainer_create_handler<Source: CollectionClientSource>(
    call: &ContainerCreateCall<'_>,
    source: &Source,
    supplied: Option<&Source::Asynchronous>,
    hook: &dyn file::PreCallHook,
) -> Result<records::ContainerObject, ContainerCollectionError> {
    let params = request::read_params(call.litellm_params)?;
    let ssl_verify = params.ssl_verify.as_ref();
    let client = Selected::choose(supplied, || source.async_client(ssl_verify))?;
    create_body(client.get(), &params, call, hook).await
}

pub fn container_list_handler<Source: CollectionClientSource>(
    call: &ContainerListCall<'_>,
    source: &Source,
    supplied: Option<&Source::Blocking>,
    hook: &dyn file::PreCallHook,
) -> Result<records::ContainerListResponse, ContainerCollectionError> {
    let params = request::read_params(call.litellm_params)?;
    let ssl_verify = params.ssl_verify.as_ref();
    let client = Selected::choose(supplied, || source.blocking_client(ssl_verify))?;
    poll_once(list_body(client.get(), &params, call, hook))?
}

pub async fn acontainer_list_handler<Source: CollectionClientSource>(
    call: &ContainerListCall<'_>,
    source: &Source,
    supplied: Option<&Source::Asynchronous>,
    hook: &dyn file::PreCallHook,
) -> Result<records::ContainerListResponse, ContainerCollectionError> {
    let params = request::read_params(call.litellm_params)?;
    let ssl_verify = params.ssl_verify.as_ref();
    let client = Selected::choose(supplied, || source.async_client(ssl_verify))?;
    list_body(client.get(), &params, call, hook).await
}

// container_create_handler's steps after the client: headers, base URL, transform, log, POST, then the object
async fn create_body<Client: CollectionTransport>(
    client: &Client,
    params: &request::CallParams<'_>,
    call: &ContainerCreateCall<'_>,
    hook: &dyn file::PreCallHook,
) -> Result<records::ContainerObject, ContainerCollectionError> {
    let config = call.config;
    let extra = call.extra_headers.cloned().unwrap_or_default();
    let api_key = params.api_key.as_ref().map(ExposeSecret::expose_secret);
    // the JSON content type goes in after validate_environment and before the overlay, so an extra header of that name wins
    let mut headers = config.validate_environment(&extra, api_key);
    headers.insert("Content-Type".to_owned(), JSON_CONTENT_TYPE.to_owned());
    for (name, value) in &extra {
        headers.insert(name.clone(), value.clone());
    }
    let url = config.get_complete_url(params.api_base, call.litellm_params)?;
    let context = config::ContainerRequestContext {
        litellm_params: call.litellm_params,
        headers: &headers,
    };
    let data = config.transform_container_create_request(call.name, call.params, context);
    log_create(call, hook, &url, &headers, &data)?;
    let request = CollectionRequest {
        method: transport::ContainerMethod::Post,
        url,
        headers,
        query: Vec::new(),
        body: Some(serde_json::to_vec(&data).map_err(ContainerCollectionError::Encode)?),
        timeout: Some(call.timeout.unwrap_or(DEFAULT_TIMEOUT)),
    };
    let reply = client.send(&request).await.map_err(|failure| {
        ContainerCollectionError::Provider(provider_failure(config, &failure, true))
    })?;
    post_status(config, &reply)?;
    config
        .transform_container_create_response(&reply.body)
        .map_err(ContainerCollectionError::Config)
}

// container_list_handler's steps after the client: headers, base URL, transform, log, GET, then the page
async fn list_body<Client: CollectionTransport>(
    client: &Client,
    params: &request::CallParams<'_>,
    call: &ContainerListCall<'_>,
    hook: &dyn file::PreCallHook,
) -> Result<records::ContainerListResponse, ContainerCollectionError> {
    let config = call.config;
    let headers = request::request_headers(config, call.extra_headers, params.api_key.as_ref());
    let api_base = config.get_complete_url(params.api_base, call.litellm_params)?;
    let context = config::ContainerRequestContext {
        litellm_params: call.litellm_params,
        headers: &headers,
    };
    let (url, list_params) =
        config.transform_container_list_request(&api_base, context, &call.query);
    let rendered = render_query(&list_params);
    let record = request::pre_call_record(&url, &headers, &rendered);
    let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    let logged = hook.pre_call(&mut state, &record);
    drop(state);
    logged?;
    let request = CollectionRequest {
        method: transport::ContainerMethod::Get,
        url,
        headers,
        query: rendered.into_iter().collect(),
        body: None,
        timeout: None,
    };
    let reply = client.send(&request).await.map_err(|failure| {
        ContainerCollectionError::Provider(provider_failure(config, &failure, false))
    })?;
    response::raise_for_error_status(config, &reply)?;
    config
        .transform_container_list_response(&reply.body)
        .map_err(ContainerCollectionError::Config)
}

// httpx writes a param that is not a string with str(), and a null one as the empty string
fn render_query(params: &Object) -> IndexMap<String, String> {
    params
        .iter()
        .map(|(name, value)| match value.as_str() {
            Some(text) => (name.clone(), text.to_owned()),
            None if value.is_null() => (name.clone(), String::new()),
            None => (name.clone(), value.to_string()),
        })
        .collect()
}

// LiteLLM's create pre_call: the name as input, the JSON about to be sent as the complete input, and no params
fn log_create(
    call: &ContainerCreateCall<'_>,
    hook: &dyn file::PreCallHook,
    url: &str,
    headers: &BTreeMap<String, String>,
    data: &Object,
) -> Result<(), ContainerCollectionError> {
    let encoded = serde_json::to_value(data).map_err(ContainerCollectionError::Encode)?;
    let complete = serde_json::from_value(encoded).map_err(ContainerCollectionError::Encode)?;
    let mut record = request::pre_call_record(url, headers, &IndexMap::new());
    record.input = Json::Text(call.name.to_owned());
    record.additional_args.passthrough.clear();
    record.additional_args.complete_input_dict = Some(RawBody::Object(complete));
    let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    hook.pre_call(&mut state, &record)?;
    Ok(())
}

// _handle_error's conversion: a POST timeout arrives as litellm.Timeout with status 408, every other exception a synthesized 500
fn provider_failure(
    config: &dyn config::BaseContainerConfig,
    failure: &transport::TransferError,
    posted: bool,
) -> error::BaseLlmException {
    let timed_out = matches!(
        failure,
        transport::TransferError::Transport(
            TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
        )
    );
    let status = if posted && timed_out {
        REQUEST_TIMEOUT_STATUS
    } else {
        SERVER_ERROR_STATUS
    };
    config.get_error_class(&failure.to_string(), status, &BTreeMap::new())
}

// HTTPHandler.post calls raise_for_status, so an error status arrives through _handle_error with the whole body text
fn post_status(
    config: &dyn config::BaseContainerConfig,
    reply: &transport::ContainerHttpReply,
) -> Result<(), ContainerCollectionError> {
    if (200..=299).contains(&reply.status) {
        return Ok(());
    }
    let message = str::from_utf8(&reply.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", reply.body.len()),
        str::to_owned,
    );
    let provider = config.get_error_class(&message, reply.status, &reply.headers);
    Err(ContainerCollectionError::Provider(provider))
}

#[path = "container_collection_http/tests/create.rs"]
mod create;
#[path = "container_collection_http/tests/list.rs"]
mod list;
mod tests;
