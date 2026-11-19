use std::future::Future;
use std::iter;
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::stream::BoxStream;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use url::Url;

use self::facts::{Attempt, status_facts, transport_facts};
use super::config::{AuthCall, GenerateContentHttpConfig};
use super::error::{GenAiHttpError, classed, wire_failure};
use super::stream::{
    StreamInputs, StreamSettings, buffered_source, decision_json, decision_object, stream_metadata,
};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::{ClientKind, LogContext, SuppliedClient};
use crate::genai_setup::config::GenerateContentRequest;
use crate::genai_setup::params::GenerateContentLiteLLMParams;
use crate::google_genai::metadata::GenAiStreamMetadata;
use crate::google_genai::streaming::{AsyncGoogleGenAiStream, GoogleGenAiStream};

mod facts;
mod transport;

// What generate_content_handler reads off the handler it runs in: the provider config, the client pool and the
// iterator settings B3487's async stream logs through
pub struct GenerateContentDeps<'deps, C: GenerateContentHttpConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub stream: StreamSettings,
}

// The call generate_content_handler is handed, field for field
pub struct GenerateContentCall<'call> {
    pub model: &'call str,
    pub contents: &'call Json,
    pub config: &'call Object,
    pub tools: Option<&'call Json>,
    pub custom_llm_provider: &'call str,
    pub litellm_params: &'call GenerateContentLiteLLMParams,
    pub extra_headers: Option<&'call IndexMap<String, String>>,
    pub extra_body: Option<&'call Object>,
    pub system_instruction: Option<&'call Json>,
    pub timeout: Option<Duration>,
    pub stream: bool,
}

pub struct GenAiRequest {
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: Vec<u8>,
    pub timeout: Option<Duration>,
}

// httpx's stream=True response: the headers are read before the body starts, and the body stays open
pub struct StreamReply {
    pub headers: HttpHeaders,
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

pub enum WireReply {
    Buffered(UpstreamResponse),
    Streaming(StreamReply),
}

// The selected httpx client class: the sync and async forms differ in the default timeout they carry and in whether
// post keeps the response open
pub trait GenAiTransport: Send + Sync {
    fn default_timeout(&self) -> Duration;

    fn send(
        &self,
        request: GenAiRequest,
        streaming: bool,
    ) -> impl Future<Output = Result<WireReply, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client: GenAiTransport;

    fn client(&self, ssl_verify: Option<&Json>) -> Result<Self::Client, ClientError>;
}

pub struct SyncStream {
    pub iterator: GoogleGenAiStream,
    pub metadata: GenAiStreamMetadata,
}

pub struct AsyncStream {
    pub iterator: AsyncGoogleGenAiStream,
    pub metadata: GenAiStreamMetadata,
}

pub enum GenerateContentOutcome<Response> {
    Response(Response),
    Stream(Box<SyncStream>),
}

pub enum AsyncGenerateContentOutcome<Response> {
    Response(Response),
    Stream(Box<AsyncStream>),
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

struct Prepared<'borrow, 'call, C: GenerateContentHttpConfig, S: ClientSource> {
    config: &'borrow C,
    client: Selected<'borrow, S::Client>,
    mode: ClientKind,
    stream: bool,
    request: GenAiRequest,
    inputs: StreamInputs<'call>,
    settings: &'borrow StreamSettings,
}

// What a sent request still needs to become a response or an iterator
struct Wire<'borrow, 'call, C: GenerateContentHttpConfig> {
    config: &'borrow C,
    inputs: StreamInputs<'call>,
    settings: &'borrow StreamSettings,
}

enum Sent<'borrow, 'call, C: GenerateContentHttpConfig> {
    Unary(Wire<'borrow, 'call, C>, UpstreamResponse),
    SyncEvents(Wire<'borrow, 'call, C>, UpstreamResponse),
    AsyncEvents(Wire<'borrow, 'call, C>, StreamReply),
}

pub async fn generate_content<'deps, C, S>(
    deps: &GenerateContentDeps<'deps, C, S>,
    call: &GenerateContentCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<GenerateContentOutcome<C::Response>, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    let prepared = prepare(deps, ClientKind::Sync, call, log).await?;
    match send(prepared).await? {
        Sent::Unary(wire, answer) => {
            unary(wire.config, answer, log.state).map(GenerateContentOutcome::Response)
        }
        Sent::SyncEvents(wire, answer) => Ok(GenerateContentOutcome::Stream(Box::new(
            sync_stream(&wire.inputs, wire.settings, answer),
        ))),
        Sent::AsyncEvents(_, _) => Err(GenAiHttpError::UnexpectedStream),
    }
}

pub async fn async_generate_content<'deps, C, S>(
    deps: &GenerateContentDeps<'deps, C, S>,
    call: &GenerateContentCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<AsyncGenerateContentOutcome<C::Response>, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    let prepared = prepare(deps, ClientKind::Asynchronous, call, log).await?;
    match send(prepared).await? {
        Sent::Unary(wire, answer) => {
            unary(wire.config, answer, log.state).map(AsyncGenerateContentOutcome::Response)
        }
        Sent::SyncEvents(wire, answer) => {
            let body = buffered_source(answer.body);
            let stream = async_stream(&wire.inputs, wire.settings, &answer.headers, body);
            Ok(AsyncGenerateContentOutcome::Stream(Box::new(stream)))
        }
        Sent::AsyncEvents(wire, answer) => {
            let stream = async_stream(&wire.inputs, wire.settings, &answer.headers, answer.body);
            Ok(AsyncGenerateContentOutcome::Stream(Box::new(stream)))
        }
    }
}

// The handler's front half: client selection, auth, the caller's header overlay, transform, the body overlay, the
// pre-call and the wire form of the request
async fn prepare<'borrow, 'deps, 'call, C, S>(
    deps: &'borrow GenerateContentDeps<'deps, C, S>,
    mode: ClientKind,
    call: &'call GenerateContentCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Prepared<'borrow, 'call, C, S>, GenAiHttpError<C::Error>>
where
    'deps: 'borrow,
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    let client = select_client(deps, mode, call.litellm_params.ssl_verify.as_ref())?;
    let auth = deps
        .config
        .auth_token_and_url(&AuthCall {
            api_base: call.litellm_params.api_base.as_deref(),
            model: call.model,
            params: call.litellm_params,
            stream: call.stream,
        })
        .await
        .map_err(GenAiHttpError::Auth)?;
    let headers = overlay_headers(auth.headers, call.extra_headers);
    let body = request_body(deps.config, call)?;
    log_pre_call(call, &headers, &body, &auth.url, log)?;
    let request = wire_request(deps.config, &auth.url, headers, &body, call.timeout)?;
    let inputs = StreamInputs {
        model: call.model,
        provider: call.custom_llm_provider,
        body,
        params: call.litellm_params,
        api_base: auth.url,
    };
    Ok(Prepared {
        config: deps.config,
        client,
        mode,
        stream: call.stream,
        request,
        inputs,
        settings: &deps.stream,
    })
}

// The request as the wire carries it: the provider's URL, the overlaid headers and the encoded body
fn wire_request<C>(
    config: &C,
    endpoint: &str,
    headers: IndexMap<String, String>,
    body: &Object,
    timeout: Option<Duration>,
) -> Result<GenAiRequest, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let url = Url::parse(endpoint).map_err(|source| {
        GenAiHttpError::Provider(wire_failure(
            config,
            format!("the request URL {endpoint} cannot be parsed: {source}"),
        ))
    })?;
    Ok(GenAiRequest {
        url,
        headers: wire_headers(config, headers)?,
        body: encode_body(config, body)?,
        timeout,
    })
}

fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow GenerateContentDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<&Json>,
) -> Result<Selected<'borrow, S::Client>, GenAiHttpError<C::Error>>
where
    'deps: 'borrow,
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

// headers.update(extra_headers), so a caller's header beats the provider's
fn overlay_headers(
    mut headers: IndexMap<String, String>,
    extra: Option<&IndexMap<String, String>>,
) -> IndexMap<String, String> {
    if let Some(extra) = extra {
        headers.extend(extra.clone());
    }
    headers
}

// transform_generate_content_request, then data.update(extra_body)
fn request_body<C>(
    config: &C,
    call: &GenerateContentCall<'_>,
) -> Result<Object, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let transformed = config
        .transform_request(&GenerateContentRequest {
            model: call.model,
            contents: call.contents,
            tools: call.tools,
            generation_config: call.config,
            system_instruction: call.system_instruction,
        })
        .map_err(GenAiHttpError::Transform)?;
    let Json::Object(mut body) = transformed else {
        return Err(GenAiHttpError::BodyNotObject);
    };
    if let Some(extra) = call.extra_body {
        body.extend(extra.clone());
    }
    Ok(body)
}

fn log_pre_call(
    call: &GenerateContentCall<'_>,
    headers: &IndexMap<String, String>,
    body: &Object,
    api_base: &str,
    log: &mut LogContext<'_, '_>,
) -> Result<(), PrePostError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), DecisionJson::Text(value.clone()));
    }
    let record = PreCall {
        input: decision_json(call.contents),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(decision_object(body))),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)
}

fn wire_headers<C>(
    config: &C,
    headers: IndexMap<String, String>,
) -> Result<HttpHeaders, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            GenAiHttpError::Provider(wire_failure(
                config,
                format!("the {name} header name is not a header name: {source}"),
            ))
        })?;
        let header_value = HeaderValue::from_str(&value).map_err(|source| {
            GenAiHttpError::Provider(wire_failure(
                config,
                format!("the {name} header value is not a header value: {source}"),
            ))
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn encode_body<C>(config: &C, body: &Object) -> Result<Vec<u8>, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    serde_json::to_vec(body).map_err(|source| {
        GenAiHttpError::Provider(wire_failure(
            config,
            format!("the request body cannot be written as JSON: {source}"),
        ))
    })
}

// One POST, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
async fn send<'borrow, 'call, C, S>(
    prepared: Prepared<'borrow, 'call, C, S>,
) -> Result<Sent<'borrow, 'call, C>, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    let Prepared {
        config,
        client,
        mode,
        stream,
        request,
        inputs,
        settings,
    } = prepared;
    let requested = request.timeout;
    let timeout = requested.unwrap_or_else(|| client.get().default_timeout());
    let started = Instant::now();
    let streaming = stream && mode == ClientKind::Asynchronous;
    let reply = client
        .get()
        .send(request, streaming)
        .await
        .map_err(|error| {
            let attempt = Attempt {
                mode,
                timeout: requested,
                effective: timeout,
                elapsed: started.elapsed(),
            };
            GenAiHttpError::Provider(classed(config, &transport_facts(&error, &attempt)))
        })?;
    let wire = Wire {
        config,
        inputs,
        settings,
    };
    match reply {
        WireReply::Buffered(answer) => {
            let answer = successful(config, answer)?;
            if stream {
                Ok(Sent::SyncEvents(wire, answer))
            } else {
                Ok(Sent::Unary(wire, answer))
            }
        }
        WireReply::Streaming(answer) if stream => Ok(Sent::AsyncEvents(wire, answer)),
        WireReply::Streaming(_) => Err(GenAiHttpError::UnexpectedStream),
    }
}

fn successful<C>(
    config: &C,
    answer: UpstreamResponse,
) -> Result<UpstreamResponse, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    if answer.status < 400 {
        return Ok(answer);
    }
    Err(GenAiHttpError::Provider(classed(
        config,
        &status_facts(&answer),
    )))
}

// transform_generate_content_response runs outside the try, and the raw reply it read stays on the logging state
fn unary<C>(
    config: &C,
    answer: UpstreamResponse,
    state: &mut LoggingState,
) -> Result<C::Response, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
{
    let response = config
        .decode_response(&answer)
        .map_err(GenAiHttpError::Provider)?;
    state.httpx_response = Some(answer);
    Ok(response)
}

fn sync_stream(
    inputs: &StreamInputs<'_>,
    settings: &StreamSettings,
    answer: UpstreamResponse,
) -> SyncStream {
    let metadata = stream_metadata(inputs, &answer.headers);
    let iterator = GoogleGenAiStream::new(iter::once(Ok(answer.body)), settings.line_limit);
    SyncStream { iterator, metadata }
}

fn async_stream(
    inputs: &StreamInputs<'_>,
    settings: &StreamSettings,
    headers: &HttpHeaders,
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
) -> AsyncStream {
    let metadata = stream_metadata(inputs, headers);
    let iterator = AsyncGoogleGenAiStream::new(
        body,
        metadata.clone(),
        settings.logging.clone(),
        Arc::clone(&settings.clock),
        settings.line_limit,
    );
    AsyncStream { iterator, metadata }
}
