use betterlite_core::api::image_variations::call::VariationAnswer;
use betterlite_core::api::image_variations::error::{ImageVariationError, VariationException};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::{Datum, Outcome, Success};

pub(super) fn success(answer: VariationAnswer) -> Result<SdkResult, SdkHttpError> {
    let VariationAnswer::Response(response) = answer else {
        return Err(SdkHttpError::Invalid(
            "the image variation call returned no image response".to_owned(),
        ));
    };
    let data = response
        .data
        .unwrap_or_default()
        .into_iter()
        .map(|datum| Datum {
            b64_json: datum.b64_json,
            revised_prompt: datum.revised_prompt,
            url: datum.url,
        })
        .collect();
    Ok(SdkResult::ImageVariations {
        value: Box::new(Outcome {
            success: Some(Success {
                created: response.created,
                data,
            }),
            failure: None,
        }),
    })
}

// image_variation's own ValueErrors carry no status, type, param or code
pub(super) fn error_failure(error: &ImageVariationError) -> SdkResult {
    let (class, status) = match error {
        ImageVariationError::InvalidProvider { .. }
        | ImageVariationError::UnknownModelInfo { .. }
        | ImageVariationError::MissingKey { .. }
        | ImageVariationError::MissingBase { .. }
        | ImageVariationError::InvalidTimeout { .. } => ("ValueError", 0),
        ImageVariationError::Resolution(_) => ("BadRequestError", 400),
        ImageVariationError::OpenAi(source) => {
            (class_for_status(source.status_code), source.status_code)
        }
        ImageVariationError::Topaz(source) => ("Exception", source.status_code.unwrap_or(0)),
    };
    let failure = Failure {
        class: class.to_owned(),
        status,
        kind: None,
        param: None,
        code: None,
        message: error.to_string(),
    };
    failure_result(failure)
}

// aimage_variation re-raises through exception_type, so the class and its fields come from the mapping
pub(super) fn exception_failure(error: &VariationException) -> Result<SdkResult, SdkHttpError> {
    let metadata = error.class.metadata();
    let failure = Failure {
        class: class_name(&error.class)?.to_owned(),
        status: metadata.status_code,
        kind: field(metadata.error_type),
        param: field(metadata.param),
        code: Some(metadata.openai_code),
        message: error.message.clone(),
    };
    Ok(failure_result(failure))
}

// the helper's Result wraps both outcomes the same way: one outcome object under image_variations.value
fn failure_result(failure: Failure) -> SdkResult {
    SdkResult::ImageVariations {
        value: Box::new(Outcome {
            success: None,
            failure: Some(failure),
        }),
    }
}

const fn class_for_status(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500 => "InternalServerError",
        502 => "BadGatewayError",
        503 => "ServiceUnavailableError",
        504 => "Timeout",
        _ => "APIError",
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
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}

const fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
