use super::super::error::SdkHttpError;
use super::{Failure, RunOutcome};
use betterlite_core::api::decisions_send::error::visible_message;
use betterlite_core::api::decisions_send::format::DecisionsOutput;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::decisions::call::error::PrepareError;
use serde::Serialize;

// The SDK response: the shape _format_response chose, carried as its model_dump_json would
pub(super) fn output(response: DecisionsOutput) -> Result<RunOutcome, SdkHttpError> {
    let record = match response {
        DecisionsOutput::SystemOne(systemone) => serialized(systemone)?,
        DecisionsOutput::OpenAi(openai) => serialized(openai)?,
    };
    Ok(RunOutcome {
        result: Some(record),
        failure: None,
    })
}

fn serialized<T: Serialize>(response: T) -> Result<Json, SdkHttpError> {
    let record = serde_json::to_value(response).map_err(SdkHttpError::Serialize)?;
    serde_json::from_value(record).map_err(SdkHttpError::Serialize)
}

// _map_upstream_exception's result: the class and the text the exception's __init__ built
pub(super) fn mapped(class: &ErrorClass, message: String) -> Result<RunOutcome, SdkHttpError> {
    let metadata = class.metadata();
    Ok(RunOutcome {
        result: None,
        failure: Some(Failure {
            class: class_name(class)?.to_owned(),
            status: Some(metadata.status_code),
            kind: text_field(metadata.error_type),
            param: text_field(metadata.param),
            code: metadata.openai_code,
            message,
        }),
    })
}

// prepare_call raises its own LiteLLM class, before anything reaches the wire
pub(super) fn prepared(error: &PrepareError) -> Result<RunOutcome, SdkHttpError> {
    let class = error.class();
    let message = visible_message(&class);
    mapped(&class, message)
}

fn text_field(field: WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text),
        WireField::Missing(_) | WireField::Null => None,
    }
}

// The exception the SDK raises for this class, as the pinned litellm.exceptions module names it
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
