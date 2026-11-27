use std::collections::BTreeMap;

use betterlite_core::api::realtime_http_calls::error::RealtimeHttpCallsError;
use betterlite_core::api::vector_store_http_write::error::header_map;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_provider::api::realtime_http_transport::error::RealtimeHttpError;
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::{Answer, Failure, Logged, Outcome};

// What both sides hand back: the raw answer, or the exception a caller catches, plus the recorded pre-call
pub(super) fn outcome(
    answer: &Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>>,
    logged: Result<Option<Logged>, SdkHttpError>,
) -> Result<Outcome, SdkHttpError> {
    let record = logged?;
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

// The provider's own exception is the one a caller catches; nothing else this SDK raises has a Python twin
fn failure(error: &RealtimeHttpCallsError<RealtimeHttpError>) -> Result<Failure, SdkHttpError> {
    let RealtimeHttpCallsError::Dispatch(inner) = error else {
        return Err(SdkHttpError::Invalid(error.to_string()));
    };
    match inner {
        RealtimeHttpError::Provider(provider) => Ok(Failure {
            class: String::from("BaseLLMException"),
            status: Some(provider.status_code),
            message: provider.message.clone(),
            headers: provider.headers.clone(),
            synthesized: Some(provider.status_code_is_synthesized),
        }),
        RealtimeHttpError::Status(status) => Ok(Failure {
            class: String::from("MaskedHTTPStatusError"),
            status: Some(status.status_code),
            message: status.message.clone(),
            headers: status.headers.clone(),
            synthesized: None,
        }),
        RealtimeHttpError::Config(config) => Ok(plain("ValueError", config.to_string())),
        RealtimeHttpError::Timeout(_)
        | RealtimeHttpError::Transport(_)
        | RealtimeHttpError::Client(_)
        | RealtimeHttpError::Logging(_)
        | RealtimeHttpError::Sdp(_)
        | RealtimeHttpError::RequestUrl { .. }
        | RealtimeHttpError::HeaderName { .. }
        | RealtimeHttpError::HeaderValue { .. }
        | RealtimeHttpError::HeaderNotText { .. }
        | RealtimeHttpError::Encode(_) => Err(SdkHttpError::Invalid(error.to_string())),
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

// The pre-call details a caller's logger reads: the SDK's raw params and the handler's session defaults
pub(super) fn logged(state: &LoggingState) -> Result<Option<Logged>, SdkHttpError> {
    let details = &state.model_call_details;
    let Some(Json::Object(args)) = details.get("additional_args") else {
        return Ok(None);
    };
    let params = object(details, "litellm_params")?;
    Ok(Some(Logged {
        model: text(details, "model")?.to_owned(),
        custom_llm_provider: text(details, "custom_llm_provider")?.to_owned(),
        optional_params: details
            .get("optional_params")
            .cloned()
            .unwrap_or(Json::Null),
        litellm_api_base: text(params, "api_base")?.to_owned(),
        session: args.get("session").cloned(),
    }))
}

fn object<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &str,
) -> Result<&'fields IndexMap<String, Json>, SdkHttpError> {
    match fields.get(name) {
        Some(Json::Object(value)) => Ok(value),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Text(_))
        | None => Err(SdkHttpError::Invalid(format!(
            "the pre-call recorded no {name} object"
        ))),
    }
}

fn text<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &str,
) -> Result<&'fields str, SdkHttpError> {
    match fields.get(name) {
        Some(Json::Text(value)) => Ok(value.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Err(SdkHttpError::Invalid(format!(
            "the pre-call recorded no {name} text"
        ))),
    }
}
