use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::response_resources::request::ResourceRequest;
use betterlite_openai::response_resources::response::TypedReply;
use betterlite_prompts::arguments::json::Object as ResourceObject;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use url::Url;

use self::facts::ReplyView;
use self::request::{LoggedCall, log_pre_call, select_client, write_endpoint};
use super::config::{ResponsesConfig, WriteParams};
use super::error::{WriteError, handle_error};
use crate::api::vector_store_http_write::error::{ClientError, masked_status_facts};

mod facts;
mod proxied;
mod request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Delete,
    Cancel,
}

impl Operation {
    const fn http_method(self) -> Method {
        match self {
            Self::Delete => Method::DELETE,
            Self::Cancel => Method::POST,
        }
    }

    const fn client_label(self) -> &'static str {
        match self {
            Self::Delete => "delete_response",
            Self::Cancel => "cancel_response",
        }
    }
}

// LiteLLM's shared aiohttp session; betterlite's pooled client plays that part, so only its presence travels
#[derive(Debug, Clone, Copy)]
pub struct SharedSession;

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

pub struct WriteRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
}

// The selected httpx client class: the sync and async forms differ in the one the handler catches and logs about
pub trait WriteTransport: Send + Sync {
    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client: WriteTransport;

    fn client(
        &self,
        ssl_verify: Option<bool>,
        shared_session: Option<SharedSession>,
    ) -> Result<Self::Client, ClientError>;
}

pub struct WriteDeps<'deps, C: ResponsesConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub shared_session: Option<SharedSession>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct WriteTarget<'call> {
    pub response_id: &'call str,
    pub params: &'call WriteParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct DeleteCall<'call> {
    pub target: WriteTarget<'call>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub struct CancelCall<'call> {
    pub target: WriteTarget<'call>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub enum WriteCall<'call> {
    Delete(DeleteCall<'call>),
    Cancel(CancelCall<'call>),
}

pub enum WriteOutcome {
    Deleted(Box<DeleteResponseResult>),
    Cancelled(Box<TypedReply>),
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

pub struct PreparedWrite<'deps, C: ResponsesConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: WriteRequest,
    operation: Operation,
}

// The four handlers' shared front half: client selection, environment validation, the base plus encoded id, the body, the pre-call and the wire headers
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    call: &WriteCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedWrite<'borrow, C, S>, WriteError>
where
    'deps: 'borrow,
    C: ResponsesConfig,
    S: ClientSource,
{
    let (target, operation) = match call {
        WriteCall::Delete(delete) => (&delete.target, Operation::Delete),
        WriteCall::Cancel(cancel) => (&cancel.target, Operation::Cancel),
    };
    let client = select_client(deps, mode, operation, target.params.ssl_verify)?;
    let headers = handler_headers(deps.config, target, operation);
    let (api_base, resource) = write_endpoint(deps.config, target, operation)?;
    let encoded = serde_json::to_value(&resource.payload).map_err(WriteError::Encode)?;
    let payload: IndexMap<String, Json> =
        serde_json::from_value(encoded).map_err(WriteError::Encode)?;
    log_pre_call(
        &LoggedCall {
            operation,
            response_id: target.response_id,
            api_base: &api_base,
            url: &resource.url,
            headers: &headers,
            payload: &payload,
        },
        log,
    )?;
    Ok(PreparedWrite {
        config: deps.config,
        client,
        request: build_request(operation, target, &resource, &headers)?,
        operation,
    })
}

// The caller's map goes in first because B508 rewrites Authorization in it; only the delete handler sets the media type itself
fn handler_headers<C>(
    config: &C,
    target: &WriteTarget<'_>,
    operation: Operation,
) -> BTreeMap<String, String>
where
    C: ResponsesConfig,
{
    let mut headers = config.validate_environment(
        target.extra_headers.cloned().unwrap_or_default(),
        target.params,
    );
    if matches!(operation, Operation::Delete) {
        headers
            .entry(String::from("Content-Type"))
            .or_insert_with(|| String::from("application/json"));
    }
    headers
}

fn build_request(
    operation: Operation,
    target: &WriteTarget<'_>,
    resource: &ResourceRequest,
    headers: &BTreeMap<String, String>,
) -> Result<WriteRequest, WriteError> {
    Ok(WriteRequest {
        method: operation.http_method(),
        url: request_url(&resource.url)?,
        headers: wire_headers(headers)?,
        body: wire_body(operation, &resource.payload)?,
        timeout: target.timeout,
    })
}

fn wire_headers(headers: &BTreeMap<String, String>) -> Result<HttpHeaders, WriteError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| WriteError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| WriteError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// The delete handlers send no JSON argument for the empty mapping; cancel always posts its transformed data
fn wire_body(operation: Operation, payload: &ResourceObject) -> Result<RequestBody, WriteError> {
    if matches!(operation, Operation::Delete) && payload.is_empty() {
        return Ok(RequestBody::Empty);
    }
    serde_json::to_vec(payload)
        .map(RequestBody::Json)
        .map_err(WriteError::Encode)
}

fn request_url(text: &str) -> Result<Url, WriteError> {
    Url::parse(text).map_err(|source| WriteError::RequestUrl {
        url: text.to_owned(),
        source,
    })
}

// One send, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
pub async fn send<C, S>(prepared: PreparedWrite<'_, C, S>) -> Result<WriteOutcome, WriteError>
where
    C: ResponsesConfig,
    S: ClientSource,
{
    let PreparedWrite {
        config,
        client,
        request,
        operation,
    } = prepared;
    let reply = client.get().send(request).await.map_err(|error| {
        WriteError::Provider(handle_error(&facts::transport_facts(&error), config))
    })?;
    if reply.status < 400 {
        let view = ReplyView::new(&reply)?;
        let raw = view.raw();
        return match operation {
            Operation::Delete => config
                .delete_response(&raw)
                .map(|record| WriteOutcome::Deleted(Box::new(record)))
                .map_err(WriteError::Resource),
            Operation::Cancel => config
                .cancel_response(&raw)
                .map(|typed| WriteOutcome::Cancelled(Box::new(typed)))
                .map_err(WriteError::Resource),
        };
    }
    Err(WriteError::Provider(handle_error(
        &masked_status_facts(&reply),
        config,
    )))
}
