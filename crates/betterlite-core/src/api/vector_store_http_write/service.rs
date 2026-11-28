use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
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

use self::facts::Sent;
use self::request::{call_timeout, select_client, update_body, validated_headers, write_endpoint};
use super::config::{WriteConfig, WriteParams};
use super::error::{ClientError, WriteError, handle_error, status_facts};
use super::params::UpdateParams;

pub(crate) mod facts;
mod proxied;
mod request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Update,
    Delete,
}

impl Operation {
    const fn http_method(self) -> Method {
        match self {
            Self::Update => Method::POST,
            Self::Delete => Method::DELETE,
        }
    }
}

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

// The selected httpx client class: the sync and async forms differ in the timeout they carry and the one they catch
pub trait WriteTransport: Send + Sync {
    fn default_timeout(&self) -> Duration;

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Self::Client, ClientError>;
}

pub struct WriteDeps<'deps, C: WriteConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct WriteTarget<'call> {
    pub vector_store_id: &'call str,
    pub params: &'call WriteParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct UpdateCall<'call> {
    pub target: WriteTarget<'call>,
    pub optional_params: &'call UpdateParams,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub struct DeleteCall<'call> {
    pub target: WriteTarget<'call>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub enum WriteCall<'call> {
    Update(UpdateCall<'call>),
    Delete(DeleteCall<'call>),
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

pub struct PreparedWrite<'deps, C: WriteConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: WriteRequest,
    operation: Operation,
    mode: ClientKind,
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
    C: WriteConfig,
    S: ClientSource,
{
    let (target, operation) = match call {
        WriteCall::Update(update) => (&update.target, Operation::Update),
        WriteCall::Delete(delete) => (&delete.target, Operation::Delete),
    };
    let client = select_client(deps, mode, target.params.ssl_verify)?;
    let headers = validated_headers(deps.config, target);
    let (api_base, url) = write_endpoint(deps.config, target)?;
    let payload = update_body(call)?;
    log_pre_call(&api_base, &headers, payload.as_ref(), log)?;
    Ok(PreparedWrite {
        config: deps.config,
        client,
        request: WriteRequest {
            method: operation.http_method(),
            url,
            headers: wire_headers(&headers)?,
            body: wire_body(payload.as_ref())?,
            timeout: call_timeout(mode, target.timeout),
        },
        operation,
        mode,
    })
}

fn log_pre_call(
    api_base: &str,
    headers: &OpenAiHeaders,
    body: Option<&IndexMap<String, Json>>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), WriteError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let complete = body.cloned().map(RawBody::Object);
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: complete,
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

fn wire_headers(headers: &OpenAiHeaders) -> Result<HttpHeaders, WriteError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| WriteError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value = HeaderValue::from_str(value.expose_secret()).map_err(|source| {
            WriteError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn wire_body(body: Option<&IndexMap<String, Json>>) -> Result<RequestBody, WriteError> {
    let Some(payload) = body else {
        return Ok(RequestBody::Empty);
    };
    serde_json::to_vec(payload)
        .map(RequestBody::Json)
        .map_err(WriteError::Encode)
}

// One send, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
pub async fn send<C, S>(prepared: PreparedWrite<'_, C, S>) -> Result<Json, WriteError>
where
    C: WriteConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let PreparedWrite {
        config,
        client,
        request,
        operation,
        mode,
    } = prepared;
    let timeout = request
        .timeout
        .unwrap_or_else(|| client.get().default_timeout());
    let started = Instant::now();
    let reply = client.get().send(request).await;
    // the sync handlers pass no timeout, so their message shows none; the async one shows the deadline it ran with
    let sent = Sent {
        mode,
        operation,
        timeout: match mode {
            ClientKind::Sync => None,
            ClientKind::Asynchronous => Some(timeout),
        },
        elapsed: started.elapsed(),
    };
    let response = reply.map_err(|error| {
        WriteError::Provider(handle_error(
            &facts::transport_facts(&error, &sent),
            Some(config),
        ))
    })?;
    answer(&response, operation, config)
}

// The reply's own step: under 400 an update's reply is transformed and a delete's decoded, and 400 or more becomes the provider's error
fn answer<C>(reply: &UpstreamResponse, operation: Operation, config: &C) -> Result<Json, WriteError>
where
    C: WriteConfig,
{
    if reply.status < 400 {
        return match operation {
            Operation::Update => config
                .transform_create_response(&facts::http_response(reply))
                .map_err(WriteError::Provider),
            Operation::Delete => serde_json::from_slice(&reply.body).map_err(WriteError::Reply),
        };
    }
    Err(WriteError::Provider(handle_error(
        &status_facts(reply),
        Some(config),
    )))
}
