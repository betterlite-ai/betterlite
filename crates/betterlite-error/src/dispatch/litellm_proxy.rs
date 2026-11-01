use std::collections::BTreeMap;

use super::RawException;
use crate::error::details::RateLimitDetails;
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::presence::Field;
use crate::facts::{ErrorCode, ProviderErrorBody};
use crate::openai::body::OpenaiErrorBody;
use crate::parse::{self, LitellmExceptionFacts};
use crate::rate_limit::RateLimitCategory;

pub(super) enum LitellmProxyOutcome {
    Raised(ErrorClass),
    NoMatch,
    Unconstructible,
}

fn proxy_body(body: Option<&OpenaiErrorBody>) -> Option<ProviderErrorBody> {
    let fields = body?;
    Some(ProviderErrorBody {
        message: fields.message.clone().map_or(Field::Missing, Field::Value),
        error: Field::Missing,
        code: fields
            .code
            .clone()
            .map_or(Field::Missing, |code| Field::Value(ErrorCode::Text(code))),
    })
}

// extract_and_raise_litellm_exception: a regex match on "litellm.\w+Error" names the exception to construct
pub(super) fn litellm_proxy_branch(raw: &RawException, error_str: &str) -> LitellmProxyOutcome {
    let body = proxy_body(raw.body.as_ref());
    match parse::extract_exception(None, error_str, &raw.model, &raw.custom_llm_provider, body) {
        Ok(Some(facts)) => LitellmProxyOutcome::Raised(classify_facts(&facts)),
        Ok(None) => LitellmProxyOutcome::NoMatch,
        Err(_) => LitellmProxyOutcome::Unconstructible,
    }
}

fn body_code_string(body: &ProviderErrorBody) -> Option<String> {
    match body.code.as_ref()? {
        ErrorCode::Number(number) => Some(number.to_string()),
        ErrorCode::Text(text) => Some(text.clone()),
    }
}

// maps the 16 litellm.*Error names extract_exception can ever return Ok(Some(_)) for to their ErrorClass family
pub(super) fn classify_facts(facts: &LitellmExceptionFacts) -> ErrorClass {
    let body_code = facts.body.as_ref().and_then(body_code_string);
    let context = ErrorContext {
        message: facts.message.clone(),
        model: Some(facts.model.clone()),
        provider: Some(facts.provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code,
    };
    match facts.name.as_str() {
        "AuthenticationError" => ErrorClass::Authentication(Box::new(context)),
        "BadGatewayError" => ErrorClass::BadGateway(Box::new(context)),
        "ContextWindowExceededError" => ErrorClass::ContextWindowExceeded(Box::new(context)),
        "NotFoundError" => ErrorClass::NotFound(Box::new(context)),
        "PermissionDeniedError" => ErrorClass::PermissionDenied(Box::new(context)),
        "ServiceUnavailableError" => ErrorClass::ServiceUnavailable(Box::new(context)),
        "UnprocessableEntityError" => ErrorClass::UnprocessableEntity(Box::new(context)),
        "UnsupportedParamsError" => ErrorClass::UnsupportedParams(Box::new(context)),
        "ContentPolicyViolationError" => ErrorClass::ContentPolicyViolation(Box::new(context)),
        "InternalServerError" => ErrorClass::InternalServer(Box::new(context)),
        "RateLimitError" => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        "APIConnectionError" => ErrorClass::ApiConnection(Box::new(context)),
        "APIResponseValidationError" => ErrorClass::ApiResponseValidation(Box::new(context)),
        // BadRequestError, ImageFetchError and the deprecated InvalidRequestError alias share one family
        "BadRequestError" | "ImageFetchError" | "InvalidRequestError" => {
            ErrorClass::BadRequest(Box::new(context))
        }
        _ => ErrorClass::Unclassified(Box::new(context)),
    }
}
