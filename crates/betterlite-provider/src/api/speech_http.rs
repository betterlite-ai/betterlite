use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use http::header::HeaderValue;
use http::{HeaderMap, Method};
use secrecy::SecretString;
use url::Url;

use self::error::{SpeechError, handle_error};
use self::facts::Sent;
use self::request::{
    Payload, call_endpoint, payload, request_headers, select_client, validated_headers,
    wire_headers,
};
use crate::api::speech_provider_contract::{SpeechHeaders, SpeechRequestInput, TextToSpeechConfig};

pub mod error;
mod facts;
mod logging;
mod proxied;
mod request;
mod tests;

// httpx sends a str body with no content type; B173's byte body always carries one, so the text type is added
const SSML_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

// The client class the handler selects, which decides the timeout it applies and the one it reports
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Asynchronous,
}

// The three litellm_params keys the handler reads before the provider config sees the rest of the dict
pub struct SpeechParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub ssl_verify: Option<bool>,
}

// The handler's arguments: the speech input and the caller's overrides
pub struct SpeechCall<'call> {
    pub model: &'call str,
    pub input: &'call str,
    pub voice: Option<&'call str>,
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
    pub extra_headers: Option<&'call SpeechHeaders>,
    pub params: &'call SpeechParams,
    pub timeout: Option<Duration>,
}

// A post ready for the selected client, with the timeout the caller passed and nothing else
pub struct SpeechWireRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
}

// The selected httpx client class: its own default timeout and the send it performs
pub trait SpeechTransport: Send + Sync {
    fn default_timeout(&self) -> Duration;

    fn send(
        &self,
        request: SpeechWireRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub struct SuppliedClient<'client, Client> {
    pub kind: Mode,
    pub transport: &'client Client,
}

pub struct SpeechDeps<'deps, S: ClientSource> {
    pub config: &'deps dyn TextToSpeechConfig,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
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

// LiteLLM's HttpxBinaryResponseContent: the provider bytes and both B206 hidden header maps
pub struct SpeechBinaryResponse {
    pub content: BinaryContent,
    pub hidden: ProviderHeaderProjection,
}

struct SpeechTarget {
    url: Url,
    headers: HeaderMap,
    timeout: Option<Duration>,
}

pub struct PreparedSpeech<'deps, S: ClientSource> {
    config: &'deps dyn TextToSpeechConfig,
    client: Selected<'deps, S::Client>,
    target: SpeechTarget,
    payload: Payload,
    mode: Mode,
}

// Everything before the send: client selection, header validation, the completed URL, the transform, the pre-call and the wire request
pub fn prepare<'borrow, 'deps, S>(
    deps: &'borrow SpeechDeps<'deps, S>,
    mode: Mode,
    call: &SpeechCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedSpeech<'borrow, S>, SpeechError>
where
    'deps: 'borrow,
    S: ClientSource,
{
    let client = select_client(deps, mode, call.params.ssl_verify)?;
    let mut headers = validated_headers(deps.config, call);
    let (api_base, url) = call_endpoint(deps.config, call)?;
    let data = deps.config.speech_request(&SpeechRequestInput {
        model: call.model,
        input: call.input,
        voice: call.voice,
        optional_params: call.optional_params,
        litellm_params: call.litellm_params,
        headers: &headers,
    });
    request_headers(&mut headers, &data);
    logging::log_pre_call(call.input, &api_base, &headers, &data, log)?;
    Ok(PreparedSpeech {
        config: deps.config,
        client,
        target: SpeechTarget {
            url,
            headers: wire_headers(&headers)?,
            timeout: call.timeout,
        },
        payload: payload(&data),
        mode,
    })
}

// One post, no retry: a missing body, a transport failure and a non-2xx status all become provider errors
pub async fn send<S>(
    prepared: PreparedSpeech<'_, S>,
    model: &str,
) -> Result<SpeechBinaryResponse, SpeechError>
where
    S: ClientSource<Client: SpeechTransport>,
{
    let PreparedSpeech {
        config,
        client,
        target,
        payload,
        mode,
    } = prepared;
    let timeout = target.timeout;
    let effective = timeout.unwrap_or_else(|| client.get().default_timeout());
    let started = Instant::now();
    let reply = client
        .get()
        .send(SpeechWireRequest {
            method: Method::POST,
            url: target.url,
            headers: target.headers,
            body: wire_body(payload, config)?,
            timeout,
        })
        .await;
    let sent = Sent {
        mode,
        passed: timeout,
        effective,
        elapsed: started.elapsed(),
    };
    let response = reply.map_err(|error| {
        SpeechError::Provider(handle_error(&facts::transport(&error, &sent), config))
    })?;
    if !(200..300).contains(&response.status) {
        let failed = facts::status(&response);
        return Err(SpeechError::Provider(handle_error(&failed, config)));
    }
    let content = config.speech_response(model, response);
    let hidden = set_provider_response_headers(&AdditionalHeaders::default(), &content.headers);
    Ok(SpeechBinaryResponse { content, hidden })
}

// The missing body is the ValueError raised inside the send try, so it reaches the same converter as a failed post
fn wire_body(
    payload: Payload,
    config: &dyn TextToSpeechConfig,
) -> Result<RequestBody, SpeechError> {
    match payload {
        Payload::Dict(fields) => Ok(RequestBody::Json(
            serde_json::to_vec(&fields).map_err(SpeechError::Encode)?,
        )),
        Payload::Ssml(text) => Ok(RequestBody::Bytes {
            content_type: HeaderValue::from_static(SSML_CONTENT_TYPE),
            data: text.into_bytes(),
        }),
        Payload::Missing => Err(SpeechError::Provider(handle_error(
            &facts::missing_body(),
            config,
        ))),
    }
}

// ast-grep-ignore: no-padded-one-line-functions - both public entry points forward here, and this body is the pipeline itself
async fn execute<S>(
    deps: &SpeechDeps<'_, S>,
    mode: Mode,
    call: &SpeechCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<SpeechBinaryResponse, SpeechError>
where
    S: ClientSource<Client: SpeechTransport>,
{
    let prepared = prepare(deps, mode, call, log)?;
    send(prepared, call.model).await
}

impl<S> SpeechDeps<'_, S>
where
    S: ClientSource<Client: SpeechTransport>,
{
    // ast-grep-ignore: no-one-line-functions - the public sync entry point forwards to the shared pipeline with its client class
    pub async fn text_to_speech_handler(
        &self,
        call: &SpeechCall<'_>,
        log: &mut LogContext<'_, '_>,
    ) -> Result<SpeechBinaryResponse, SpeechError> {
        execute(self, Mode::Sync, call, log).await
    }

    // ast-grep-ignore: no-one-line-functions - the public async entry point forwards to the shared pipeline with its client class
    pub async fn async_text_to_speech_handler(
        &self,
        call: &SpeechCall<'_>,
        log: &mut LogContext<'_, '_>,
    ) -> Result<SpeechBinaryResponse, SpeechError> {
        execute(self, Mode::Asynchronous, call, log).await
    }
}
