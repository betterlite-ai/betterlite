use std::collections::BTreeMap;

use super::{RawException, RawExceptionBody};
use crate::error::details::{RateLimitDetails, ResponseDetails};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::presence::Field;
use crate::facts::{ErrorCode, ProviderErrorBody};
use crate::parse::{self, LitellmExceptionFacts};
use crate::rate_limit::RateLimitCategory;

pub(super) enum LitellmProxyOutcome {
    Raised(ErrorClass),
    NoMatch,
    Unconstructible,
}

fn proxy_body(body: Option<&RawExceptionBody>) -> Option<ProviderErrorBody> {
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
        Ok(Some(facts)) => LitellmProxyOutcome::Raised(classify_facts(&facts, raw.status_code)),
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

fn proxy_context(facts: &LitellmExceptionFacts, body_code: Option<String>) -> ErrorContext {
    ErrorContext {
        message: facts.message.clone(),
        model: Some(facts.model.clone()),
        provider: Some(facts.provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code,
    }
}

// extract_and_raise_litellm_exception passes a response to every constructor below except InvalidRequestError, which keeps BadRequest's class default
const RESPONSE_NAMES: &[&str] = &[
    "AuthenticationError",
    "NotFoundError",
    "PermissionDeniedError",
    "UnprocessableEntityError",
    "ContextWindowExceededError",
    "ContentPolicyViolationError",
    "BadRequestError",
    "ImageFetchError",
];

fn response_name_class(name: &str, context: ErrorContext, status_code: Option<u16>) -> ErrorClass {
    let status = if name == "InvalidRequestError" {
        None
    } else {
        status_code
    };
    match name {
        "AuthenticationError" => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status)))
        }
        "NotFoundError" => {
            ErrorClass::NotFound(Box::new(ResponseDetails::with_status(context, status)))
        }
        "PermissionDeniedError" => {
            ErrorClass::PermissionDenied(Box::new(ResponseDetails::with_status(context, status)))
        }
        "UnprocessableEntityError" => {
            ErrorClass::UnprocessableEntity(Box::new(ResponseDetails::with_status(context, status)))
        }
        "ContextWindowExceededError" => ErrorClass::ContextWindowExceeded(Box::new(
            ResponseDetails::with_status(context, status),
        )),
        "ContentPolicyViolationError" => ErrorClass::ContentPolicyViolation(Box::new(
            ResponseDetails::with_status(context, status),
        )),
        _ => ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status))),
    }
}

type PlainBuild = fn(ErrorContext) -> ErrorClass;

const PLAIN_NAMES: &[(&str, PlainBuild)] = &[
    ("BadGatewayError", |context| {
        ErrorClass::BadGateway(Box::new(context))
    }),
    ("ServiceUnavailableError", |context| {
        ErrorClass::ServiceUnavailable(Box::new(context))
    }),
    ("UnsupportedParamsError", |context| {
        ErrorClass::UnsupportedParams(Box::new(context))
    }),
    ("InternalServerError", |context| {
        ErrorClass::InternalServer(Box::new(context))
    }),
    ("APIConnectionError", |context| {
        ErrorClass::ApiConnection(Box::new(context))
    }),
    ("APIResponseValidationError", |context| {
        ErrorClass::ApiResponseValidation(Box::new(context))
    }),
];

fn plain_class(name: &str, context: ErrorContext) -> ErrorClass {
    if name == "RateLimitError" {
        return ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        }));
    }
    match PLAIN_NAMES.iter().find(|(known, _)| *known == name) {
        Some((_, build)) => build(context),
        None => ErrorClass::Unclassified(Box::new(context)),
    }
}

fn name_class(name: &str, context: ErrorContext, status_code: Option<u16>) -> ErrorClass {
    if name == "InvalidRequestError" || RESPONSE_NAMES.contains(&name) {
        return response_name_class(name, context, status_code);
    }
    plain_class(name, context)
}

// maps the 16 litellm.*Error names extract_exception can ever return Ok(Some(_)) for to their ErrorClass family
pub(super) fn classify_facts(
    facts: &LitellmExceptionFacts,
    status_code: Option<u16>,
) -> ErrorClass {
    let body_code = facts.body.as_ref().and_then(body_code_string);
    let context = proxy_context(facts, body_code);
    name_class(&facts.name, context, status_code)
}
