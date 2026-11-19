use betterlite_core::api::assistant_threads::error::{ThreadError, ThreadFailure};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};

// The synchronous facade raises the SDK's exception itself, so its attributes are the projection.
pub(super) fn sync_failure(error: &ThreadError) -> SdkResult {
    let sdk = error.sdk_failure();
    SdkResult::Failure(Failure {
        class: sdk.class.to_owned(),
        status: sdk.status,
        kind: Some(sdk.kind),
        param: Some(sdk.param),
        code: Some(sdk.code),
        message: sdk.message,
    })
}

// acreate_thread and aget_thread pass the SDK error through LiteLLM's exception_type.
pub(super) fn async_failure(failure: &ThreadFailure) -> Result<SdkResult, SdkHttpError> {
    let metadata = failure.class.metadata();
    let (class, message) = class_name(&failure.class)?;
    Ok(SdkResult::Failure(Failure {
        class: class.to_owned(),
        status: metadata.status_code,
        kind: Some(field(metadata.error_type)),
        param: Some(field(metadata.param)),
        code: Some(metadata.openai_code),
        message: message.to_owned(),
    }))
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// Only the classes the pinned SDK raises as APIError subclasses have a projection here.
fn class_name(class: &ErrorClass) -> Result<(&'static str, &str), SdkHttpError> {
    match class {
        ErrorClass::Authentication(details) => Ok(("AuthenticationError", &details.message)),
        ErrorClass::NotFound(details) => Ok(("NotFoundError", &details.message)),
        ErrorClass::PermissionDenied(details) => Ok(("PermissionDeniedError", &details.message)),
        ErrorClass::BadRequest(details) => Ok(("BadRequestError", &details.message)),
        ErrorClass::UnprocessableEntity(details) => {
            Ok(("UnprocessableEntityError", &details.message))
        }
        ErrorClass::ContextWindowExceeded(details) => {
            Ok(("ContextWindowExceededError", &details.message))
        }
        ErrorClass::ContentPolicyViolation(details) => {
            Ok(("ContentPolicyViolationError", &details.message))
        }
        ErrorClass::ServiceUnavailable(context) => {
            Ok(("ServiceUnavailableError", &context.message))
        }
        ErrorClass::BadGateway(context) => Ok(("BadGatewayError", &context.message)),
        ErrorClass::InternalServer(context) => Ok(("InternalServerError", &context.message)),
        ErrorClass::Timeout(details) => Ok(("Timeout", &details.context.message)),
        ErrorClass::RateLimit(details) => Ok(("RateLimitError", &details.context.message)),
        ErrorClass::Api(details) => Ok(("APIError", &details.context.message)),
        ErrorClass::ApiConnection(context) => Ok(("APIConnectionError", &context.message)),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}
