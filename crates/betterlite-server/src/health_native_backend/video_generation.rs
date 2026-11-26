use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_generation_call::call::{
    GenerationCallEnvironment, VideoGenerationCaller,
};
use betterlite_core::api::video_generation_call::error::VideoGenerationFailure;
use betterlite_core::api::video_generation_call::handler::VideoGenerationHandling;
use betterlite_core::api::video_generation_call::providers::VideoGenerationProviders;
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::videos_http_transport::video_config::Params;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::call_helpers::headers::provider_response_headers;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::de::{DeserializeOwned, Error as _};

use super::seams::VideoGenerationProbeCall;
use crate::health_probe_modes::calls::{HealthProbeReply, ProbeCall};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::HealthProbeLogging;

const API_KEY_KEY: &str = "api_key";
const PROMPT_KEY: &str = "prompt";
const TIMEOUT_KEY: &str = "timeout";
const STRING: &str = "a string";
const NUMBER: &str = "a number";

// what one health check takes from the server around the call: the request resolution its providers read
pub trait VideoGenerationSetup: Send + Sync {
    fn resolution(&self) -> ResolutionContext<'_>;
}

// B3338's avideo_generation behind B4440's video_generation mode: B4440's call arguments in, the reply's headers out
pub struct VideoGenerationProbe<Providers, Handler, Setup> {
    pub caller: VideoGenerationCaller<Providers, Handler>,
    pub setup: Setup,
    pub clock: fn() -> DateTime<FixedOffset>,
}

#[async_trait]
impl<Providers, Handler, Setup> VideoGenerationProbeCall
    for VideoGenerationProbe<Providers, Handler, Setup>
where
    Providers: VideoGenerationProviders,
    Handler: VideoGenerationHandling<Providers::Config, Providers::Source>,
    Setup: VideoGenerationSetup,
{
    async fn generate(&self, call: ProbeCall) -> Result<HealthProbeReply, HealthProbeError> {
        let request = generation_request(&call)?;
        let resolution = self.setup.resolution();
        let (mut logging, shared) = take_logging(&call, self.placeholder()?);
        let sink = QuietSink;
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.caller.redactor,
            log: &sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: (self.clock)(),
        };
        let outcome = self
            .caller
            .avideo_generation(
                request,
                None,
                &resolution,
                GenerationCallEnvironment {
                    logging: &mut logging,
                    env: &env,
                },
            )
            .await;
        restore_logging(shared, logging);
        let object = outcome.map_err(generation_failure)?;
        Ok(generation_reply(&object))
    }
}

impl<Providers, Handler, Setup> VideoGenerationProbe<Providers, Handler, Setup>
where
    Providers: VideoGenerationProviders,
    Setup: VideoGenerationSetup,
{
    // the stand-in a shared logging state is swapped with while the call holds the real one
    fn placeholder(&self) -> Result<LoggingState, HealthProbeError> {
        let mut correlation = CorrelationContext::default();
        let setup = CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        };
        let init = LoggingInit {
            model: String::new(),
            call_type: String::new(),
            start_time: (self.clock)(),
            litellm_call_id: String::new(),
            function_id: String::new(),
            messages: None,
            stream: None,
            litellm_trace_id: None,
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        };
        LoggingState::new(init, setup, &self.caller.redactor, String::new, || {
            Duration::ZERO
        })
        .map_err(|source| HealthProbeError::Logging { source })
    }
}

// ahealth_check's keywords bind to the entry's named arguments; the provider settings stay in the loose map
fn generation_request(call: &ProbeCall) -> Result<VideoGenerationRequest, HealthProbeError> {
    let mut parameters = call.parameters.clone();
    let prompt = take(&mut parameters, PROMPT_KEY)?.ok_or_else(|| missing(PROMPT_KEY, STRING))?;
    let mut request = VideoGenerationRequest::new(prompt);
    request.model = take(&mut parameters, "model")?;
    request.custom_llm_provider = take(&mut parameters, "custom_llm_provider")?;
    request.seconds = take(&mut parameters, "seconds")?;
    request.size = take(&mut parameters, "size")?;
    request.user = take(&mut parameters, "user")?;
    request.extra_headers = take(&mut parameters, "extra_headers")?;
    request.extra_query = take(&mut parameters, "extra_query")?;
    request.extra_body = take(&mut parameters, "extra_body")?;
    apply_timeout(&mut request, &mut parameters)?;
    if let Some(key) = &call.api_key {
        let value = Json::Text(key.expose_secret().to_owned());
        parameters.insert(API_KEY_KEY.to_owned(), value);
    }
    request.kwargs = parameters;
    Ok(request)
}

// timeout is a named argument with the signature's 600 default: absent keeps it, null reaches it as none
fn apply_timeout(
    request: &mut VideoGenerationRequest,
    parameters: &mut Params,
) -> Result<(), HealthProbeError> {
    match parameters.shift_remove(TIMEOUT_KEY) {
        None => {}
        Some(Json::Null) => request.timeout = None,
        Some(Json::Number(number)) => {
            request.timeout = Some(
                number
                    .as_f64()
                    .ok_or_else(|| missing(TIMEOUT_KEY, NUMBER))?,
            );
        }
        Some(_) => return Err(missing(TIMEOUT_KEY, NUMBER)),
    }
    Ok(())
}

// a declared argument takes the JSON type its name says; absent and null both leave it unset
fn take<T>(parameters: &mut Params, field: &'static str) -> Result<Option<T>, HealthProbeError>
where
    T: DeserializeOwned,
{
    let Some(value) = parameters.shift_remove(field) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let encoded =
        serde_json::to_string(&value).map_err(|source| parameter_failure(field, source))?;
    serde_json::from_str(&encoded)
        .map(Some)
        .map_err(|source| parameter_failure(field, source))
}

fn parameter_failure(field: &'static str, source: serde_json::Error) -> HealthProbeError {
    HealthProbeError::ParameterValue {
        key: field.to_owned(),
        source,
    }
}

fn missing(field: &'static str, expected: &str) -> HealthProbeError {
    parameter_failure(
        field,
        serde_json::Error::custom(format!("{field} must be {expected}")),
    )
}

// ahealth_check reads _hidden_params.headers off the answer; the entry keeps every provider header it was handed
#[must_use]
pub fn generation_reply(object: &VideoObject) -> HealthProbeReply {
    let hidden = Json::Object(object.hidden_params.clone());
    let headers = provider_response_headers(Some(&hidden)).unwrap_or_default();
    HealthProbeReply {
        hidden_headers: headers.into_iter().collect(),
    }
}

// the entry updates a logging state by &mut, so the health object's state is swapped out for the call and back
fn take_logging(
    call: &ProbeCall,
    placeholder: LoggingState,
) -> (LoggingState, Option<Arc<Mutex<LoggingState>>>) {
    let Some(shared) = call
        .logging
        .as_deref()
        .and_then(HealthProbeLogging::logging_state)
    else {
        return (placeholder, None);
    };
    let mut guard = shared.lock().unwrap_or_else(PoisonError::into_inner);
    let state = std::mem::replace(&mut *guard, placeholder);
    drop(guard);
    (state, Some(shared))
}

fn restore_logging(shared: Option<Arc<Mutex<LoggingState>>>, state: LoggingState) {
    let Some(shared) = shared else {
        return;
    };
    let mut guard = shared.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = state;
}

fn generation_failure(error: VideoGenerationFailure) -> HealthProbeError {
    HealthProbeError::Backend {
        class: error.class.clone(),
        message: error.to_string(),
        source: Box::new(error),
    }
}

struct QuietSink;

impl LogSink for QuietSink {
    fn emit(&self, _record: LogRecord) {}
}

mod tests;
