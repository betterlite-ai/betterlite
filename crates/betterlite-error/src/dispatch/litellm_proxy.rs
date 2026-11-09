use std::collections::BTreeMap;

use super::{RawException, RawExceptionBody};
use crate::error::details::{RateLimitDetails, ResponseDetails};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::presence::Field;
use crate::facts::{ErrorCode, ProviderErrorBody};
use crate::parse::constructors::ExceptionName;
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
        Ok(Some(facts)) => {
            LitellmProxyOutcome::Raised(classify_facts(&facts, raw.status_code, raw.body.as_ref()))
        }
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

// the body goes only to the exceptions whose constructor takes one, which leaves facts.body set; the others keep the class default
fn proxy_context(
    facts: &LitellmExceptionFacts,
    body_code: Option<String>,
    raw_body: Option<&RawExceptionBody>,
) -> ErrorContext {
    let forwarded = raw_body.filter(|_| facts.body.is_some());
    ErrorContext {
        message: facts.message.clone(),
        model: Some(facts.model.clone()),
        provider: Some(facts.provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: forwarded.and_then(|body| body.kind.clone()),
        body_param: forwarded.and_then(|body| body.param.clone()),
        body_code,
    }
}

// extract_and_raise_litellm_exception passes a response to every constructor below except InvalidRequestError, which keeps BadRequest's class default
pub(super) fn classify_facts(
    facts: &LitellmExceptionFacts,
    status_code: Option<u16>,
    raw_body: Option<&RawExceptionBody>,
) -> ErrorClass {
    use ErrorClass as Class;
    use ExceptionName as Name;
    use ResponseDetails as Response;

    let body_code = facts.body.as_ref().and_then(body_code_string);
    let context = proxy_context(facts, body_code, raw_body);
    let status = status_code;
    match facts.name {
        Name::Authentication => {
            Class::Authentication(Box::new(Response::with_status(context, status)))
        }
        Name::NotFound => Class::NotFound(Box::new(Response::with_status(context, status))),
        Name::PermissionDenied => {
            Class::PermissionDenied(Box::new(Response::with_status(context, status)))
        }
        Name::UnprocessableEntity => {
            Class::UnprocessableEntity(Box::new(Response::with_status(context, status)))
        }
        Name::ContextWindowExceeded => {
            Class::ContextWindowExceeded(Box::new(Response::with_status(context, status)))
        }
        Name::ContentPolicyViolation => {
            Class::ContentPolicyViolation(Box::new(Response::with_status(context, status)))
        }
        Name::BadRequest | Name::ImageFetch => {
            Class::BadRequest(Box::new(Response::with_status(context, status)))
        }
        Name::InvalidRequest => Class::BadRequest(Box::new(Response::with_status(context, None))),
        Name::BadGateway => Class::BadGateway(Box::new(context)),
        Name::ServiceUnavailable => Class::ServiceUnavailable(Box::new(context)),
        Name::UnsupportedParams => Class::UnsupportedParams(Box::new(context)),
        Name::InternalServer => Class::InternalServer(Box::new(context)),
        Name::ApiConnection => Class::ApiConnection(Box::new(context)),
        Name::ApiResponseValidation => Class::ApiResponseValidation(Box::new(context)),
        Name::RateLimit => Class::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
    }
}
