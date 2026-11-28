use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_azure::speech::dispatch::{
    DispatchCall, HandlerFailure, HandlerRequest, SpeechEnvironment, SpeechGlobals,
    TextToSpeechHandler, dispatch_text_to_speech,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::speech_http::error::SpeechError;
use crate::api::speech_http::{
    LogContext, SpeechBinaryResponse, SpeechCall, SpeechDeps, SpeechParams, SpeechTransport,
    SpeechWireRequest,
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
    C: Sync,
    K: Sync,
    T: TranscriptionTransport,
    L: Send,
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

    async fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> Result<Self::Output, HandlerFailure> {
        let optional_json = serde_json::to_value(request.optional_params)
            .map_err(|error| encode_failure(error.to_string()))?;
        let optional_params = serde_json::from_value(optional_json)
            .map_err(|error| encode_failure(error.to_string()))?;
        let litellm_json = serde_json::to_value(request.litellm_params)
            .map_err(|error| encode_failure(error.to_string()))?;
        let litellm_params = serde_json::from_value(litellm_json)
            .map_err(|error| encode_failure(error.to_string()))?;
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

fn encode_failure(message: String) -> HandlerFailure {
    HandlerFailure {
        class: String::from("Exception"),
        status: None,
        message,
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> SpeechHeaders {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// text_to_speech_handler reads ssl_verify off litellm_params; the media transport owns TLS, so the value never reaches the wire
#[cfg_attr(test, mutants::skip)]
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
    let message = error.to_string();
    let (class, status) = match error {
        SpeechError::Provider(provider) => {
            (String::from("APIStatusError"), Some(provider.status_code))
        }
        SpeechError::Client(_)
        | SpeechError::Logging(_)
        | SpeechError::CompleteUrl(_)
        | SpeechError::RequestUrl { .. }
        | SpeechError::HeaderName { .. }
        | SpeechError::HeaderValue { .. }
        | SpeechError::Encode(_) => (String::from("Exception"), None),
    };
    HandlerFailure {
        class,
        status,
        message,
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

    async fn send(&self, request: SpeechWireRequest) -> Result<UpstreamResponse, TransportError> {
        let outbound = OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: request.body,
            deadline: request.timeout.unwrap_or_else(|| self.default_timeout()),
        };
        self.transport.send(outbound).await
    }
}
