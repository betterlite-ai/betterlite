use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_http_write::error::header_map;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_provider::api::realtime_http_transport::TransportDeps;
use betterlite_provider::api::realtime_http_transport::calls::{CallsCall, calls_post};
use betterlite_provider::api::realtime_http_transport::decode::SdpError;
use betterlite_provider::api::realtime_http_transport::error::{
    ProviderError, RealtimeHttpError, StatusFailure,
};
use betterlite_provider::api::realtime_http_transport::session::{
    SessionCall, SessionEndpoint, session_post,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{AdapterConfig, source};
use super::{Answer, Call, Failure, Handler, Logged, Outcome};

const INIT: &str = r#"{"model":"realtime","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// The production handlers over the mock's base: the same provider config, pooled client and pre-call a request runs
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&config, env)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let mut state = logging_state(&redactor)?;
    let sink = Sink;
    let record_env = recording_env(&redactor, &sink, now()?);
    let provider = call.echo_config.then_some(AdapterConfig);
    let deps = TransportDeps {
        config: provider.as_ref(),
        source: &origin,
        supplied: None,
        limits: ApiLimits::default(),
    };
    let caller_headers = call
        .extra_headers
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut log = LogContext {
        state: &mut state,
        env: &record_env,
    };
    let answer = exchange(call, api_base, &deps, &caller_headers, &mut log).await;
    let reported = outcome(&answer, &state)?;
    Ok(SdkResult::RealtimeHttpTransport {
        value: Box::new(reported),
    })
}

// The recording environment: the defaults a case brings, with content and key info off and the fixed clock
fn recording_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

async fn exchange(
    call: &Call,
    api_base: &str,
    deps: &TransportDeps<'_, AdapterConfig>,
    caller_headers: &BTreeMap<String, String>,
    log: &mut LogContext<'_, '_>,
) -> Result<UpstreamResponse, RealtimeHttpError> {
    if call.handler == Handler::Calls {
        return calls(call, api_base, deps, caller_headers, log).await;
    }
    session(call, api_base, deps, caller_headers, log).await
}

async fn session(
    call: &Call,
    api_base: &str,
    deps: &TransportDeps<'_, AdapterConfig>,
    caller_headers: &BTreeMap<String, String>,
    log: &mut LogContext<'_, '_>,
) -> Result<UpstreamResponse, RealtimeHttpError> {
    let endpoint = if call.handler == Handler::ClientSecrets {
        SessionEndpoint::ClientSecrets
    } else {
        SessionEndpoint::TranscriptionSessions
    };
    let data = call.request_data.clone().unwrap_or_default();
    session_post(
        deps,
        &SessionCall {
            endpoint,
            api_base,
            api_key: &call.api_key,
            request_data: &data,
            model: call.model.as_deref(),
            extra_headers: Some(caller_headers),
            api_version: call.api_version.as_deref(),
            timeout: Some(Duration::from_secs(call.timeout_seconds)),
        },
        log,
    )
    .await
}

async fn calls(
    call: &Call,
    api_base: &str,
    deps: &TransportDeps<'_, AdapterConfig>,
    caller_headers: &BTreeMap<String, String>,
    log: &mut LogContext<'_, '_>,
) -> Result<UpstreamResponse, RealtimeHttpError> {
    calls_post(
        deps,
        &CallsCall {
            api_base,
            ephemeral_key: call.ephemeral_key.as_deref().unwrap_or_default(),
            sdp: call.sdp.as_slice(),
            model: call.model.as_deref(),
            session: call.session.as_ref(),
            extra_headers: Some(caller_headers),
            api_version: call.api_version.as_deref(),
            timeout: Some(Duration::from_secs(call.timeout_seconds)),
        },
        log,
    )
    .await
}

// What both sides hand back: the raw answer, or the exception a caller would catch, plus the recorded pre-call
fn outcome(
    answer: &Result<UpstreamResponse, RealtimeHttpError>,
    state: &LoggingState,
) -> Result<Outcome, SdkHttpError> {
    let record = logged(state)?;
    Ok(match answer {
        Ok(response) => Outcome {
            result: Some(Answer {
                status: response.status,
                headers: header_map(&response.headers),
                body: response.body.clone(),
            }),
            failure: None,
            logged: record,
        },
        Err(error) => Outcome {
            result: None,
            failure: Some(failure(error)?),
            logged: record,
        },
    })
}

// A provider failure is the exception class a caller catches; anything betterlite alone produces is not a parity answer
fn failure(error: &RealtimeHttpError) -> Result<Failure, SdkHttpError> {
    match error {
        RealtimeHttpError::Provider(inner) => Ok(provider_failure(inner)),
        RealtimeHttpError::Status(inner) => Ok(status_failure(inner)),
        RealtimeHttpError::Sdp(inner) => sdp_failure(inner, error),
        RealtimeHttpError::Config(inner) => Ok(plain("ValueError", inner.to_string())),
        RealtimeHttpError::Timeout(_)
        | RealtimeHttpError::Transport(_)
        | RealtimeHttpError::Client(_)
        | RealtimeHttpError::Logging(_)
        | RealtimeHttpError::RequestUrl { .. }
        | RealtimeHttpError::HeaderName { .. }
        | RealtimeHttpError::HeaderValue { .. }
        | RealtimeHttpError::HeaderNotText { .. }
        | RealtimeHttpError::Encode(_) => Err(SdkHttpError::Invalid(error.to_string())),
    }
}

fn provider_failure(error: &ProviderError) -> Failure {
    Failure {
        class: String::from("BaseLLMException"),
        status: Some(error.status_code),
        message: error.message.clone(),
        headers: error.headers.clone(),
        synthesized: Some(error.status_code_is_synthesized),
        ..Failure::default()
    }
}

fn status_failure(error: &StatusFailure) -> Failure {
    Failure {
        class: String::from("MaskedHTTPStatusError"),
        status: Some(error.status_code),
        message: error.message.clone(),
        headers: error.headers.clone(),
        ..Failure::default()
    }
}

// The handler strings the offer before the pre-call, so a decode failure is the Python codec's own exception
fn sdp_failure(error: &SdpError, whole: &RealtimeHttpError) -> Result<Failure, SdkHttpError> {
    match error {
        SdpError::Utf8(_) => Ok(plain("UnicodeDecodeError", whole.to_string())),
        SdpError::Limit(_) => Err(SdkHttpError::Invalid(whole.to_string())),
    }
}

fn plain(class: &str, message: String) -> Failure {
    Failure {
        class: String::from(class),
        message,
        ..Failure::default()
    }
}

// The pre-call details a caller's logger reads; headers keep their names, since the shared redaction masks their values
fn logged(state: &LoggingState) -> Result<Option<Logged>, SdkHttpError> {
    let details = &state.model_call_details;
    let Some(Json::Object(args)) = details.get("additional_args") else {
        return Ok(None);
    };
    let Some(Json::Object(headers)) = args.get("headers") else {
        return Err(SdkHttpError::Invalid(String::from(
            "the pre-call recorded no headers",
        )));
    };
    let Some(Json::Object(params)) = details.get("litellm_params") else {
        return Err(SdkHttpError::Invalid(String::from(
            "the pre-call recorded no litellm_params",
        )));
    };
    let mut header_names: Vec<String> = headers.keys().cloned().collect();
    header_names.sort();
    Ok(Some(Logged {
        input: recorded(details, "input")?,
        api_base: text(args, "api_base")?,
        header_names,
        complete_input_dict: args.get("complete_input_dict").cloned(),
        session: args.get("session").cloned(),
        litellm_api_base: text(params, "api_base")?,
    }))
}

fn recorded(fields: &IndexMap<String, Json>, name: &str) -> Result<Json, SdkHttpError> {
    fields
        .get(name)
        .cloned()
        .ok_or_else(|| SdkHttpError::Invalid(format!("the pre-call recorded no {name}")))
}

fn text(fields: &IndexMap<String, Json>, name: &str) -> Result<String, SdkHttpError> {
    let Some(Json::Text(value)) = fields.get(name) else {
        return Err(SdkHttpError::Invalid(format!(
            "the pre-call recorded no {name} text"
        )));
    };
    Ok(value.clone())
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn now() -> Result<DateTime<FixedOffset>, SdkHttpError> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit =
        serde_json::from_str(INIT).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .map_err(SdkHttpError::LoggingState)
}
