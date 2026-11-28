use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_http::proxy::{pool::PoolError, transport_error::TransportError};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::{config, error, records};
use betterlite_openai::containers::container_file_http::request::{self, SslVerify};
use betterlite_openai::containers::container_file_http::transport::{self, ClientSource, Selected};
use betterlite_openai::containers::container_file_http::{self as file, response};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::ExposeSecret;
use thiserror::Error;

// LiteLLM declares 600 s for both handlers; its POST turns a timeout into litellm.Timeout's 408 and _handle_error synthesizes a 500 for anything else
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const REQUEST_TIMEOUT_STATUS: u16 = 408;
const SERVER_ERROR_STATUS: u16 = 500;

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
pub struct CollectionBase<'call, Source: ClientSource> {
    pub config: &'call dyn config::BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
    pub source: &'call Source,
    pub blocking: Option<&'call Source::Blocking>,
    pub asynchronous: Option<&'call Source::Asynchronous>,
    pub hook: &'call dyn file::PreCallHook,
}

pub struct ContainerCreateCall<'call, Source: ClientSource> {
    pub base: CollectionBase<'call, Source>,
    pub name: &'call str,
    pub params: &'call Object,
}
pub struct ContainerListCall<'call, Source: ClientSource> {
    pub base: CollectionBase<'call, Source>,
    pub query: config::ContainerListQuery<'call>,
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
    Encode(#[from] serde_json::Error),
    #[error("the blocking path was asked to wait for a client that never answered")]
    Suspended,
}

type Chosen<'client, 'params, Client> = (Selected<'client, Client>, request::CallParams<'params>);
// The params come first because the client build needs their ssl_verify, so a bad one fails before any client exists
fn pick<'call, 'supplied, Source, Client>(
    base: &CollectionBase<'call, Source>,
    supplied: Option<&'supplied Client>,
    build: fn(&Source, Option<&SslVerify>) -> Result<Client, PoolError>,
) -> Result<Chosen<'supplied, 'call, Client>, ContainerCollectionError>
where
    Source: ClientSource,
{
    let params = request::read_params(base.litellm_params)?;
    let client = Selected::choose(supplied, || build(base.source, params.ssl_verify.as_ref()))?;
    Ok((client, params))
}

fn poll_once<Running>(future: Running) -> Result<Running::Output, ContainerCollectionError>
where
    Running: Future,
{
    let mut pinned = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match pinned.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => Ok(outcome),
        Poll::Pending => Err(ContainerCollectionError::Suspended),
    }
}

// The four entry points differ only in the call, the reply and the awaited body, so one template spells them
macro_rules! collection_handlers {
    ($($blocking:ident, $asynchronous:ident, $call:ident, $reply:path, $body:ident;)*) => {
        $(
            pub fn $blocking<Source>(
                call: &$call<'_, Source>,
            ) -> Result<$reply, ContainerCollectionError>
            where
                Source: ClientSource<Blocking: CollectionTransport, Asynchronous: CollectionTransport>,
            {
                let (client, params) =
                    pick(&call.base, call.base.blocking, Source::blocking_client)?;
                poll_once($body(client.get(), &params, call))?
            }

            pub async fn $asynchronous<Source>(
                call: &$call<'_, Source>,
            ) -> Result<$reply, ContainerCollectionError>
            where
                Source: ClientSource<Blocking: CollectionTransport, Asynchronous: CollectionTransport>,
            {
                let (client, params) =
                    pick(&call.base, call.base.asynchronous, Source::async_client)?;
                $body(client.get(), &params, call).await
            }
        )*
    };
}

collection_handlers! {
    container_create_handler, acontainer_create_handler, ContainerCreateCall, records::ContainerObject, create_body;
    container_list_handler, acontainer_list_handler, ContainerListCall, records::ContainerListResponse, list_body;
}

async fn create_body<Client, Source>(
    client: &Client,
    params: &request::CallParams<'_>,
    call: &ContainerCreateCall<'_, Source>,
) -> Result<records::ContainerObject, ContainerCollectionError>
where
    Client: CollectionTransport,
    Source: ClientSource<Blocking: CollectionTransport, Asynchronous: CollectionTransport>,
{
    let config = call.base.config;
    let extra = call.base.extra_headers.cloned().unwrap_or_default();
    let api_key = params.api_key.as_ref().map(ExposeSecret::expose_secret);
    let mut headers = config.validate_environment(&extra, api_key);
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers.extend(extra);
    let url = config.get_complete_url(params.api_base, call.base.litellm_params)?;
    let context = config::ContainerRequestContext {
        litellm_params: call.base.litellm_params,
        headers: &headers,
    };
    let data = config.transform_container_create_request(call.name, call.params, context);
    let mut record = request::pre_call_record(&url, &headers, &IndexMap::new());
    record.input = Json::Text(call.name.to_owned());
    record.additional_args.passthrough.clear();
    let logged = serde_json::from_value(serde_json::to_value(&data)?)?;
    record.additional_args.complete_input_dict = Some(RawBody::Object(logged));
    log_record(call.base.logging, call.base.hook, &record)?;
    let encoded = serde_json::to_vec(&data).map_err(ContainerCollectionError::Encode)?;
    let request = CollectionRequest {
        method: transport::ContainerMethod::Post,
        url,
        headers,
        query: Vec::new(),
        body: Some(encoded),
        timeout: Some(call.base.timeout.unwrap_or(DEFAULT_TIMEOUT)),
    };
    let reply = send(client, config, &request, true).await?;
    // LiteLLM's POST calls raise_for_status, so every non-2xx reply is an error; the list GET has no such call
    if !(200..=299).contains(&reply.status) {
        let message = str::from_utf8(&reply.body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", reply.body.len()),
            str::to_owned,
        );
        let provider = config.get_error_class(&message, reply.status, &reply.headers);
        return Err(ContainerCollectionError::Provider(provider));
    }
    Ok(config.transform_container_create_response(&reply.body)?)
}

async fn list_body<Client, Source>(
    client: &Client,
    params: &request::CallParams<'_>,
    call: &ContainerListCall<'_, Source>,
) -> Result<records::ContainerListResponse, ContainerCollectionError>
where
    Client: CollectionTransport,
    Source: ClientSource<Blocking: CollectionTransport, Asynchronous: CollectionTransport>,
{
    let config = call.base.config;
    let headers =
        request::request_headers(config, call.base.extra_headers, params.api_key.as_ref());
    let base_url = config.get_complete_url(params.api_base, call.base.litellm_params)?;
    let context = config::ContainerRequestContext {
        litellm_params: call.base.litellm_params,
        headers: &headers,
    };
    let (url, query) = config.transform_container_list_request(&base_url, context, &call.query);
    let rendered: IndexMap<String, String> = query
        .iter()
        .map(|(name, value)| {
            let text = match value.as_str() {
                Some(text) => text.to_owned(),
                None if value.is_null() => String::new(),
                None => value.to_string(),
            };
            (name.clone(), text)
        })
        .collect();
    let record = request::pre_call_record(&url, &headers, &rendered);
    log_record(call.base.logging, call.base.hook, &record)?;
    let request = CollectionRequest {
        method: transport::ContainerMethod::Get,
        url,
        headers,
        query: rendered.into_iter().collect(),
        body: None,
        timeout: None,
    };
    let reply = send(client, config, &request, false).await?;
    response::raise_for_error_status(config, &reply)?;
    Ok(config.transform_container_list_response(&reply.body)?)
}

// Every exception from the send converts through _handle_error, with the status a POST timeout keeps and any other failure's 500
async fn send<Client>(
    client: &Client,
    config: &dyn config::BaseContainerConfig,
    request: &CollectionRequest,
    posted: bool,
) -> Result<transport::ContainerHttpReply, ContainerCollectionError>
where
    Client: CollectionTransport,
{
    client.send(request).await.map_err(|failure| {
        let timed_out = matches!(
            &failure,
            transport::TransferError::Transport(
                TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
            )
        );
        let status = if posted && timed_out {
            REQUEST_TIMEOUT_STATUS
        } else {
            SERVER_ERROR_STATUS
        };
        ContainerCollectionError::Provider(config.get_error_class(
            &failure.to_string(),
            status,
            &BTreeMap::new(),
        ))
    })
}

// B249's pre_call through the hook, with the log guard released before the result is read
fn log_record(
    logging: &Mutex<LoggingState>,
    hook: &dyn file::PreCallHook,
    record: &PreCall,
) -> Result<(), ContainerCollectionError> {
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    let logged = hook.pre_call(&mut state, record);
    drop(state);
    logged.map_err(ContainerCollectionError::from)
}

#[path = "container_collection_http/tests/create.rs"]
mod create;
#[path = "container_collection_http/tests/list.rs"]
mod list;
#[path = "container_collection_http/tests.rs"]
mod tests;
