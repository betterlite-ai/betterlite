use std::future::{Future, ready};
use std::time::{Duration, Instant};

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::StreamExt as _;
use futures_util::stream;
use futures_util::stream::BoxStream;
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use url::Url;

use self::facts::{Attempt, status_facts, transport_facts};
use self::outcome::{async_stream, sync_stream};
use self::prepare::prepare;
use super::config::GenerateContentHttpConfig;
use super::error::{GenAiHttpError, classed};
use super::stream::{StreamInputs, StreamSettings};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::{ClientKind, LogContext, SuppliedClient};
use crate::genai_setup::params::GenerateContentLiteLLMParams;
use crate::google_genai::metadata::GenAiStreamMetadata;
use crate::google_genai::streaming::{AsyncGoogleGenAiStream, GoogleGenAiStream};

mod facts;
mod outcome;
mod prepare;
mod transport;

// What generate_content_handler reads off the handler it runs in: the provider config, the client pool and the stream settings
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

// The selected httpx client class: the sync and async forms differ in their default timeout and their streaming
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
    match send(prepare(deps, ClientKind::Sync, call, log).await?).await? {
        Sent::Unary(wire, answer) => {
            unary(wire.config, answer, log.state).map(GenerateContentOutcome::Response)
        }
        Sent::SyncEvents(wire, answer) => Ok(GenerateContentOutcome::Stream(Box::new(
            sync_stream(&wire.inputs, wire.settings, answer),
        ))),
        Sent::AsyncEvents(_, _) => Err(GenAiHttpError::UnexpectedStream),
    }
}

pub async fn async_generate_content<'context, Client, Source>(
    deps: &GenerateContentDeps<'context, Client, Source>,
    call: &GenerateContentCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<AsyncGenerateContentOutcome<Client::Response>, GenAiHttpError<Client::Error>>
where
    Client: GenerateContentHttpConfig,
    Source: ClientSource,
{
    let staged = prepare(deps, ClientKind::Asynchronous, call, log).await?;
    let answered = send(staged).await?;
    match answered {
        Sent::Unary(wire, reply) => {
            unary(wire.config, reply, log.state).map(AsyncGenerateContentOutcome::Response)
        }
        Sent::SyncEvents(wire, reply) => {
            Ok(AsyncGenerateContentOutcome::Stream(Box::new(async_stream(
                &wire.inputs,
                wire.settings,
                &reply.headers,
                stream::once(ready(Ok::<Vec<u8>, HttpError>(reply.body))).boxed(),
            ))))
        }
        Sent::AsyncEvents(wire, reply) => Ok(AsyncGenerateContentOutcome::Stream(Box::new(
            async_stream(&wire.inputs, wire.settings, &reply.headers, reply.body),
        ))),
    }
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
