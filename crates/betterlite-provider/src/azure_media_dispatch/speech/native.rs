use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_azure::speech::dispatch::{
    DispatchCall, HandlerFailure, HandlerRequest, SpeechEnvironment, SpeechGlobals,
    TextToSpeechHandler, dispatch_text_to_speech,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::speech_http::error::{ClientError, SpeechError};
use crate::api::speech_http::{
    ClientSource, LogContext, SpeechBinaryResponse, SpeechCall, SpeechDeps, SpeechParams,
    SpeechTransport, SpeechWireRequest,
};
use crate::api::speech_provider_contract::SpeechHeaders;

use super::super::call::NativeSsmlOperation;
use super::super::error::AzureMediaError;
use super::super::runtime::MediaRuntime;
use super::config::AvaSpeechConfig;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

// AzureAVATextToSpeechConfig.dispatch_text_to_speech: resolve the call, then fill the injected handler port with B2452's handler.
pub(crate) async fn native<C, K, T, L>(
    operation: NativeSsmlOperation,
    runtime: &mut MediaRuntime<'_, C, K, T, L>,
) -> Result<SpeechBinaryResponse, AzureMediaError>
where
    T: TranscriptionTransport,
{
    // the caller resolves the module globals; only the call's own params and the environment stay in play
    let globals = SpeechGlobals::default();
    let environment = SpeechEnvironment {
        globals: &globals,
        environment: runtime.environment,
    };
    let call = DispatchCall {
        model: &operation.model,
        input: &operation.input,
        voice: operation.voice.as_ref(),
        optional_params: &operation.optional_params,
        litellm_params: operation.litellm_params,
        api_base: operation.api_base,
        api_key: operation.api_key,
        timeout: operation.timeout,
        extra_headers: operation.extra_headers.as_ref(),
        aspeech: operation.aspeech,
        environment: &environment,
    };
    let source = RuntimeSource {
        transport: runtime.transport,
    };
    let mut log = LogContext {
        state: &mut *runtime.speech_state,
        env: runtime.speech_env,
    };
    let mut bridge = Bridge {
        source: &source,
        config: AvaSpeechConfig,
        log: &mut log,
    };
    Ok(dispatch_text_to_speech(call, &mut bridge).await?)
}

// The port the leaf calls: B2452's handler over the media transport, with the B249 pre-call the leaf logged through.
struct Bridge<'log, 'state, 'env, T> {
    source: &'log RuntimeSource<'log, T>,
    config: AvaSpeechConfig,
    log: &'log mut LogContext<'state, 'env>,
}

impl<T> TextToSpeechHandler for Bridge<'_, '_, '_, T>
where
    T: TranscriptionTransport,
{
    type Output = SpeechBinaryResponse;

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<Self::Output, HandlerFailure>> + Send {
        async move {
            let optional_params = object_of(request.optional_params)?;
            let litellm_params = object_of(request.litellm_params)?;
            let params = SpeechParams {
                api_key: request.api_key.map(SecretString::from),
                api_base: request.api_base.map(str::to_owned),
                ssl_verify: ssl_verify(request.litellm_params),
            };
            let extra_headers = request.extra_headers.map(header_map);
            let call = SpeechCall {
                model: request.model,
                input: request.input,
                voice: request.voice,
                optional_params: &optional_params,
                litellm_params: &litellm_params,
                extra_headers: extra_headers.as_ref(),
                params: &params,
                timeout: request.timeout,
            };
            let deps = SpeechDeps {
                config: &self.config,
                source: self.source,
                supplied: None,
            };
            let answered = if request.is_async {
                deps.async_text_to_speech_handler(&call, &mut *self.log)
                    .await
            } else {
                deps.text_to_speech_handler(&call, &mut *self.log).await
            };
            answered.map_err(speech_failure)
        }
    }
}

// The handler's params are untyped JSON on the Python side; the leaf hands typed records, so they are re-read as a JSON object.
fn object_of(fields: &IndexMap<String, Json>) -> Result<Object, HandlerFailure> {
    let value = serde_json::to_value(fields).map_err(encode_failure)?;
    serde_json::from_value(value).map_err(encode_failure)
}

fn encode_failure(source: serde_json::Error) -> HandlerFailure {
    HandlerFailure {
        class: String::from("Exception"),
        status: None,
        message: source.to_string(),
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> SpeechHeaders {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// text_to_speech_handler reads ssl_verify off litellm_params; anything but a bool is left to the client default
fn ssl_verify(params: &IndexMap<String, Json>) -> Option<bool> {
    match params.get("ssl_verify") {
        Some(Json::Bool(value)) => Some(*value),
        None
        | Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn speech_failure(error: SpeechError) -> HandlerFailure {
    match &error {
        SpeechError::Provider(provider) => HandlerFailure {
            class: String::from("APIStatusError"),
            status: Some(provider.status_code),
            message: error.to_string(),
        },
        SpeechError::Client(_)
        | SpeechError::Logging(_)
        | SpeechError::CompleteUrl(_)
        | SpeechError::RequestUrl { .. }
        | SpeechError::HeaderName { .. }
        | SpeechError::HeaderValue { .. }
        | SpeechError::Encode(_) => HandlerFailure {
            class: String::from("Exception"),
            status: None,
            message: error.to_string(),
        },
    }
}

struct RuntimeSource<'transport, T> {
    transport: &'transport T,
}

struct RuntimeClient<'transport, T> {
    transport: &'transport T,
}

impl<'transport, T> ClientSource for RuntimeSource<'transport, T>
where
    T: TranscriptionTransport,
{
    type Client = RuntimeClient<'transport, T>;

    fn client(&self, _ssl_verify: Option<bool>) -> Result<Self::Client, ClientError> {
        Ok(RuntimeClient {
            transport: self.transport,
        })
    }
}

// The httpx client class B2452 sends through: one post over the media transport, with the SDK's default timeout.
impl<T> SpeechTransport for RuntimeClient<'_, T>
where
    T: TranscriptionTransport,
{
    fn default_timeout(&self) -> Duration {
        DEFAULT_TIMEOUT
    }

    fn send(
        &self,
        request: SpeechWireRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        async move {
            let outbound = OutboundRequest {
                method: request.method,
                url: request.url,
                query: Vec::new(),
                headers: request.headers,
                body: request.body,
                deadline: request.timeout.unwrap_or(DEFAULT_TIMEOUT),
            };
            self.transport.send(outbound).await
        }
    }
}
