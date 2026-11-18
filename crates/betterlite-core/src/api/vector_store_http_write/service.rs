use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http::url_path::encode_segment;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use self::facts::{Sent, handle_reply, handle_transport};
use super::config::{WriteConfig, WriteParams};
use super::error::{ClientError, WriteError};
use super::params::UpdateParams;

mod facts;

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

impl WriteTransport for ProxiedClient {
    fn default_timeout(&self) -> Duration {
        self.limits.read_timeout
    }

    async fn send(&self, request: WriteRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        self.send_following(OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: request.body,
            deadline,
        })
        .await?
        .buffered(self.limits.body_bytes)
        .await
    }
}

pub trait ClientSource: Send + Sync {
    type Client: WriteTransport;

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
    // the delete handlers take extra_body and never read it, so it stays on the call without reaching the body
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
    fn get(&self) -> &Client {
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
pub fn prepare<'borrow, 'deps: 'borrow, C: WriteConfig, S: ClientSource>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    call: &WriteCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedWrite<'borrow, C, S>, WriteError> {
    let (target, operation) = match call {
        WriteCall::Update(update) => (&update.target, Operation::Update),
        WriteCall::Delete(delete) => (&delete.target, Operation::Delete),
    };
    let client = select_client(deps, mode, target.params.ssl_verify)?;
    let mut caller_headers = OpenAiHeaders::new();
    for (name, value) in target.extra_headers.into_iter().flatten() {
        caller_headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    let headers = deps
        .config
        .validate_environment(caller_headers, target.params);
    let api_base = deps
        .config
        .complete_url(target.params)
        .map_err(WriteError::Url)?;
    let encoded =
        encode_segment(target.vector_store_id, "vector_store_id").map_err(WriteError::Segment)?;
    // The literal slash plus one encoded id lands after any query string, exactly as LiteLLM's f-string does
    let text = format!("{api_base}/{encoded}");
    let url = Url::parse(&text).map_err(|source| WriteError::RequestUrl { url: text, source })?;
    let body = match call {
        WriteCall::Update(update) => Some(update.optional_params.request_body(update.extra_body)?),
        WriteCall::Delete(_) => None,
    };
    log_pre_call(&api_base, &headers, body.as_ref(), log)?;
    let method = match operation {
        Operation::Update => Method::POST,
        Operation::Delete => Method::DELETE,
    };
    // LiteLLM's sync calls pass no timeout, leaving the selected client's own deadline in effect
    let timeout = match mode {
        ClientKind::Asynchronous => target.timeout,
        ClientKind::Sync => None,
    };
    let body = match &body {
        Some(body) => RequestBody::Json(serde_json::to_vec(body).map_err(WriteError::Encode)?),
        None => RequestBody::Empty,
    };
    Ok(PreparedWrite {
        config: deps.config,
        client,
        request: WriteRequest {
            method,
            url,
            headers: wire_headers(&headers)?,
            body,
            timeout,
        },
        operation,
        mode,
    })
}

fn select_client<'borrow, 'deps: 'borrow, C: WriteConfig, S: ClientSource>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, WriteError> {
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
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

// One send, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
pub async fn send<C: WriteConfig, S: ClientSource>(
    prepared: PreparedWrite<'_, C, S>,
) -> Result<Json, WriteError> {
    let PreparedWrite {
        config,
        client,
        request,
        operation,
        mode,
    } = prepared;
    let timeout = request.timeout.unwrap_or(client.get().default_timeout());
    let started = Instant::now();
    let reply = client.get().send(request).await;
    let sent = Sent {
        mode,
        operation,
        timeout,
        elapsed: started.elapsed(),
    };
    let reply =
        reply.map_err(|error| WriteError::Provider(handle_transport(&error, &sent, config)))?;
    if reply.status < 400 {
        return match operation {
            Operation::Update => config
                .transform_create_response(&facts::http_response(&reply))
                .map_err(WriteError::Provider),
            Operation::Delete => serde_json::from_slice(&reply.body).map_err(WriteError::Reply),
        };
    }
    Err(WriteError::Provider(handle_reply(&reply, config)))
}
