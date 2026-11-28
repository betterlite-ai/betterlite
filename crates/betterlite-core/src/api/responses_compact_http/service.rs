use std::future::Future;
use std::str::from_utf8;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as LoggedJson;
use betterlite_openai::response_events::{RawResponse, ResponsesReply};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use url::Url;

use self::facts::Sent;
use self::request::{
    endpoint_url, normalized_request, select_client, sign, transform, validated_environment,
    wire_body, wire_headers,
};
use super::body::signed_body_kwargs;
use super::config::{CompactConfig, CompactParams, Headers, SignRequest};
use super::error::{CompactError, ConfigFailure, Stage, provider_error};
use crate::api::vector_store_http_write::error::{ClientError, header_text, status_facts};

pub(crate) mod facts;
mod proxied;
mod request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

// The caller's shared client session; LiteLLM logs its identity and reuses it only in the async handler
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSession(pub(crate) u64);

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// The selected httpx client class: the sync and async forms differ in the default timeout they carry
pub trait CompactTransport: Send + Sync {
    fn default_timeout(&self) -> Duration;

    fn send(
        &self,
        request: CompactRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client;

    fn client(
        &self,
        ssl_verify: Option<bool>,
        shared_session: Option<SharedSession>,
    ) -> Result<Self::Client, ClientError>;
}

pub struct CompactDeps<'deps, C: CompactConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

// Both compact handlers' inputs; extra_body is accepted and never applied
pub struct CompactCall<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub optional_params: &'call Object,
    pub params: &'call CompactParams,
    pub extra_headers: Option<&'call Headers>,
    pub extra_body: Option<&'call Object>,
    pub timeout: Option<Duration>,
    pub shared_session: Option<SharedSession>,
}

// One POST the selected client sends, with the timeout exactly as the caller passed it
pub struct CompactRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
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

pub struct PreparedCompact<'deps, C: CompactConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: CompactRequest,
    mode: ClientKind,
}

// The handlers' shared front half: client, headers, compact URL and data, signing, body and pre-call
pub async fn prepare<'deps, C, S>(
    deps: &'deps CompactDeps<'_, C, S>,
    mode: ClientKind,
    call: &CompactCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedCompact<'deps, C, S>, CompactError>
where
    C: CompactConfig,
    S: ClientSource<Client: CompactTransport>,
{
    let client = select_client(deps, mode, call)?;
    let (headers, api_base) = validated_environment(deps.config, call)?;
    let transformed = transform(deps.config, call, &api_base, &headers)?;
    let data = normalized_request(deps.config, transformed.payload)?;
    let signing = SignRequest {
        headers: &headers,
        optional_params: call.params,
        request_data: &data,
        api_base: &transformed.url,
        api_key: call.params.api_key.as_ref(),
        model: Some(call.model),
    };
    let signed = sign(deps.config, mode, &signing).await?;
    let body = wire_body(&signed_body_kwargs(signed.body.as_deref(), &data))?;
    log_pre_call(call.input, &transformed.url, &signed.headers, &data, log)?;
    let request = CompactRequest {
        method: Method::POST,
        url: endpoint_url(&transformed.url)?,
        headers: wire_headers(&signed.headers)?,
        body,
        timeout: call.timeout,
    };
    Ok(PreparedCompact {
        config: deps.config,
        client,
        request,
        mode,
    })
}

fn log_pre_call(
    input: &ResponsesInput,
    url: &str,
    headers: &Headers,
    data: &Object,
    log: &mut LogContext<'_, '_>,
) -> Result<(), CompactError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), LoggedJson::Text(value.clone()));
    }
    let record = PreCall {
        input: input_json(input),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(logged_object(data))),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

fn input_json(input: &ResponsesInput) -> LoggedJson {
    match input {
        ResponsesInput::Text(text) => LoggedJson::Text(text.clone()),
        ResponsesInput::Items(items) => {
            LoggedJson::List(items.iter().map(|item| logged_json(&item.0)).collect())
        }
    }
}

fn logged_object(data: &Object) -> IndexMap<String, LoggedJson> {
    data.iter()
        .map(|(name, value)| (name.clone(), logged_json(value)))
        .collect()
}

// The two Json enums are separate types; the log keeps the same records under the logging crate's enum
fn logged_json(value: &PromptJson) -> LoggedJson {
    match value {
        PromptJson::Null => LoggedJson::Null,
        PromptJson::Bool(flag) => LoggedJson::Bool(*flag),
        PromptJson::Number(number) => LoggedJson::Number(number.clone()),
        PromptJson::Text(text) => LoggedJson::Text(text.clone()),
        PromptJson::List(items) => LoggedJson::List(items.iter().map(logged_json).collect()),
        PromptJson::Object(fields) => LoggedJson::Object(logged_object(fields)),
    }
}

// One send: a transport failure or any status outside 2xx goes through _handle_error, and only a success is transformed
pub async fn send<C, S>(
    prepared: PreparedCompact<'_, C, S>,
    log: &mut LogContext<'_, '_>,
) -> Result<ResponsesReply, CompactError>
where
    C: CompactConfig,
    S: ClientSource<Client: CompactTransport>,
{
    let PreparedCompact {
        config,
        client,
        request,
        mode,
    } = prepared;
    let requested = request.timeout;
    let effective = requested.unwrap_or_else(|| client.get().default_timeout());
    let started = Instant::now();
    let reply = client.get().send(request).await;
    let sent = Sent {
        mode,
        requested,
        effective,
        elapsed: started.elapsed(),
    };
    let answer = checked_reply(reply, &sent, config)?;
    let headers = response_headers(&answer.headers);
    let text = from_utf8(&answer.body).map_err(|source| CompactError::Step {
        stage: Stage::Response,
        source: ConfigFailure::new(source),
    })?;
    let raw = RawResponse {
        status: answer.status,
        headers: &headers,
        text,
    };
    config
        .transform_compact_response(&raw, &mut *log.state, log.env)
        .map_err(|source| CompactError::Step {
            stage: Stage::Response,
            source,
        })
}

// The send catch: a transport failure and any status outside 2xx go through _handle_error, since httpx's raise_for_status raises for a 3xx too
fn checked_reply<C>(
    reply: Result<UpstreamResponse, TransportError>,
    sent: &Sent,
    config: &C,
) -> Result<UpstreamResponse, CompactError>
where
    C: CompactConfig,
{
    let answer = reply.map_err(|error| {
        CompactError::Provider(provider_error(
            &facts::transport_facts(&error, sent),
            config,
        ))
    })?;
    if !(200..=299).contains(&answer.status) {
        return Err(CompactError::Provider(provider_error(
            &status_facts(&answer),
            config,
        )));
    }
    Ok(answer)
}

// httpx exposes dict(headers): lowercased names, one value each, non-UTF-8 described rather than mangled
fn response_headers(headers: &HttpHeaders) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}
