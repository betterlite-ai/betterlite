use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_http_write::error::header_map;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_provider::api::realtime_http_transport::TransportDeps;
use betterlite_provider::api::realtime_http_transport::calls::{CallsCall, calls_post};
use betterlite_provider::api::realtime_http_transport::decode::SdpError;
use betterlite_provider::api::realtime_http_transport::error::RealtimeHttpError;
use betterlite_provider::api::realtime_http_transport::session::{
    SessionCall, SessionEndpoint, session_post,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{AdapterConfig, Sink, logging_state, now, recording_env, source};
use super::{Answer, Call, Failure, Handler, Logged, Outcome};

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

// The handler's one POST, with the provider config, pooled client and pre-call the production paths see
enum Prepared<'call> {
    Session(SessionCall<'call>),
    Calls(CallsCall<'call>),
}

async fn exchange(
    call: &Call,
    api_base: &str,
    deps: &TransportDeps<'_, AdapterConfig>,
    caller_headers: &BTreeMap<String, String>,
    log: &mut LogContext<'_, '_>,
) -> Result<UpstreamResponse, RealtimeHttpError> {
    let data = call.request_data.clone().unwrap_or_default();
    match prepare(call, &data, api_base, caller_headers) {
        Prepared::Calls(inner) => calls_post(deps, &inner, log).await,
        Prepared::Session(inner) => session_post(deps, &inner, log).await,
    }
}

fn prepare<'call>(
    call: &'call Call,
    data: &'call IndexMap<String, Json>,
    api_base: &'call str,
    headers: &'call BTreeMap<String, String>,
) -> Prepared<'call> {
    let timeout = Some(Duration::from_secs(call.timeout_seconds));
    if call.handler == Handler::Calls {
        return Prepared::Calls(CallsCall {
            api_base,
            ephemeral_key: call.ephemeral_key.as_deref().unwrap_or_default(),
            sdp: call.sdp.as_slice(),
            model: call.model.as_deref(),
            session: call.session.as_ref(),
            extra_headers: Some(headers),
            api_version: call.api_version.as_deref(),
            timeout,
        });
    }
    let endpoint = if call.handler == Handler::ClientSecrets {
        SessionEndpoint::ClientSecrets
    } else {
        SessionEndpoint::TranscriptionSessions
    };
    Prepared::Session(SessionCall {
        endpoint,
        api_base,
        api_key: &call.api_key,
        request_data: data,
        model: call.model.as_deref(),
        extra_headers: Some(headers),
        api_version: call.api_version.as_deref(),
        timeout,
    })
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
        RealtimeHttpError::Provider(inner) => Ok(Failure {
            class: String::from("BaseLLMException"),
            status: Some(inner.status_code),
            message: inner.message.clone(),
            headers: inner.headers.clone(),
            synthesized: Some(inner.status_code_is_synthesized),
        }),
        RealtimeHttpError::Status(inner) => Ok(Failure {
            class: String::from("MaskedHTTPStatusError"),
            status: Some(inner.status_code),
            message: inner.message.clone(),
            headers: inner.headers.clone(),
            synthesized: None,
        }),
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
        status: None,
        message,
        headers: BTreeMap::new(),
        synthesized: None,
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
