use std::time::Duration;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::Redactor;

use super::super::error::SdkHttpError;
use super::super::result::{Failure, Nullable};
use super::Outcome;

// The logging init the fixture's logger starts from
const FIXTURE_INIT: &str = r#"{"model":"image-generation-test","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub(super) fn logging_state(redactor: &Redactor) -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit = serde_json::from_str(FIXTURE_INIT).map_err(SdkHttpError::Serialize)?;
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

// The exception the SDK raises for this class, as the pinned litellm.exceptions module names it
pub(super) fn failure(class: &ErrorClass, message: &str) -> Result<Outcome, SdkHttpError> {
    let metadata = class.metadata();
    Ok(Outcome {
        result: None,
        failure: Some(Failure {
            class: class_name(class)?.to_owned(),
            status: metadata.status_code,
            kind: field(metadata.error_type),
            param: field(metadata.param),
            code: Some(metadata.openai_code),
            message: message.to_owned(),
        }),
    })
}

// LiteLLMUnknownProvider is a BadRequestError built without a body, so openai.APIError leaves type, param and code null
pub(super) fn unknown_provider(message: &str) -> Outcome {
    Outcome {
        result: None,
        failure: Some(Failure {
            class: String::from("LiteLLMUnknownProvider"),
            status: 400,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: message.to_owned(),
        }),
    }
}

pub(super) fn result_json(response: &ImageResponse) -> Result<Outcome, SdkHttpError> {
    Ok(Outcome {
        result: Some(
            serde_json::from_value(
                serde_json::to_value(response).map_err(SdkHttpError::Serialize)?,
            )
            .map_err(SdkHttpError::Serialize)?,
        ),
        failure: None,
    })
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

fn class_name(class: &ErrorClass) -> Result<&'static str, SdkHttpError> {
    match class {
        ErrorClass::Authentication(_) => Ok("AuthenticationError"),
        ErrorClass::NotFound(_) => Ok("NotFoundError"),
        ErrorClass::PermissionDenied(_) => Ok("PermissionDeniedError"),
        ErrorClass::BadRequest(_) => Ok("BadRequestError"),
        ErrorClass::UnprocessableEntity(_) => Ok("UnprocessableEntityError"),
        ErrorClass::ContextWindowExceeded(_) => Ok("ContextWindowExceededError"),
        ErrorClass::ContentPolicyViolation(_) => Ok("ContentPolicyViolationError"),
        ErrorClass::ServiceUnavailable(_) => Ok("ServiceUnavailableError"),
        ErrorClass::BadGateway(_) => Ok("BadGatewayError"),
        ErrorClass::InternalServer(_) => Ok("InternalServerError"),
        ErrorClass::Timeout(_) => Ok("Timeout"),
        ErrorClass::RateLimit(_) => Ok("RateLimitError"),
        ErrorClass::Api(_) => Ok("APIError"),
        ErrorClass::ApiConnection(_) => Ok("APIConnectionError"),
        ErrorClass::UnsupportedParams(_) => Ok("UnsupportedParamsError"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}
