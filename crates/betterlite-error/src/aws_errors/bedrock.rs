use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::rate_limit::RateLimitCategory;

mod tests;

// mirrors the _map_bedrock_exception kwargs read by its branch predicates
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BedrockExceptionInput {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    ContextWindowExceeded,
    BadRequest,
    Authentication,
    PermissionDenied,
    RateLimit,
    InternalServer,
    ServiceUnavailable,
    NotFound,
    // the source status that becomes TimeoutStatus
    Timeout(u16),
}

const fn reason(family: Family) -> &'static str {
    match family {
        Family::ContextWindowExceeded => "the request exceeded the model's context window",
        Family::BadRequest => "the provider rejected the request body",
        Family::Authentication => "the provider rejected the request as unauthenticated",
        Family::PermissionDenied => "the provider denied permission for the request",
        Family::RateLimit => "the provider rate limited the request",
        Family::InternalServer => "the provider reported an internal server error",
        Family::ServiceUnavailable => "the provider is temporarily unavailable",
        Family::NotFound => "the provider could not find the requested resource",
        Family::Timeout(_) => "the request to the provider timed out",
    }
}

// bedrock either keeps the literal "bedrock" provider or forwards the caller's custom_llm_provider, depending on branch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Literal,
    Custom,
}

// _get_bedrock_mantle_context_window_message: two early-return guards before the regex ever fires
fn mantle_context_window_matches(error_str: &str) -> bool {
    if !error_str.contains("invalid_request_error") && !error_str.contains("validation_error") {
        return false;
    }
    let Ok(pattern) = regex::Regex::new(r"prompt tokens \(\d+\) exceed model maximum \(\d+\)")
    else {
        return false;
    };
    pattern.is_match(error_str)
}

// the standalone `if custom_llm_provider == "bedrock_mantle"` block, independent of the elif chain below it
fn mantle_classification(input: &BedrockExceptionInput) -> Option<(Family, Provider)> {
    if input.custom_llm_provider != "bedrock_mantle" {
        return None;
    }
    if !mantle_context_window_matches(&input.error_str) {
        return None;
    }
    Some((Family::ContextWindowExceeded, Provider::Custom))
}

const CONTEXT_WINDOW_NEEDLES: [&str; 6] = [
    "too many tokens",
    "expected maxLength:",
    "Input is too long",
    "prompt is too long",
    "prompt: length: 1..",
    "Too many input tokens",
];

type TextPredicate = fn(&str) -> bool;

// the error_str elif chain LiteLLM checks before ever looking at status_code, in source order
const TEXT_CANDIDATES: [(TextPredicate, Family, Provider); 8] = [
    (
        |error_str| {
            CONTEXT_WINDOW_NEEDLES
                .iter()
                .any(|needle| error_str.contains(needle))
        },
        Family::ContextWindowExceeded,
        Provider::Literal,
    ),
    (
        |error_str| {
            error_str.contains(
                "Conversation blocks and tool result blocks cannot be provided in the same turn.",
            )
        },
        Family::BadRequest,
        Provider::Literal,
    ),
    (
        |error_str| error_str.contains("Malformed input request"),
        Family::BadRequest,
        Provider::Literal,
    ),
    (
        |error_str| error_str.contains("A conversation must start with a user message."),
        Family::BadRequest,
        Provider::Literal,
    ),
    (
        |error_str| {
            error_str.contains("Unable to locate credentials")
                || error_str.contains("The security token included in the request is invalid")
        },
        Family::Authentication,
        Provider::Literal,
    ),
    (
        |error_str| error_str.contains("AccessDeniedException"),
        Family::PermissionDenied,
        Provider::Literal,
    ),
    (
        |error_str| {
            error_str.contains("throttlingException") || error_str.contains("ThrottlingException")
        },
        Family::RateLimit,
        Provider::Literal,
    ),
    (
        |error_str| {
            error_str.contains("Connect timeout on endpoint URL") || error_str.contains("timed out")
        },
        Family::Timeout(408),
        Provider::Literal,
    ),
];

fn classify_by_predicate(error_str: &str, status_code: Option<u16>) -> Option<(Family, Provider)> {
    if let Some((_, family, provider)) = TEXT_CANDIDATES
        .into_iter()
        .find(|(matches, ..)| matches(error_str))
    {
        return Some((family, provider));
    }
    // getattr(original_exception, "status_code", 500) == 500: an absent status_code also counts as 500 here
    if error_str.contains("Could not process image") && status_code.unwrap_or(500) == 500 {
        return Some((Family::InternalServer, Provider::Literal));
    }
    None
}

// the status_code fallback LiteLLM only reaches once every error_str predicate above has missed
const fn classify_by_status(status_code: u16) -> Option<(Family, Provider)> {
    match status_code {
        500 => Some((Family::ServiceUnavailable, Provider::Literal)),
        401 => Some((Family::Authentication, Provider::Literal)),
        400 => Some((Family::BadRequest, Provider::Literal)),
        404 => Some((Family::NotFound, Provider::Literal)),
        408 => Some((Family::Timeout(408), Provider::Custom)),
        422 => Some((Family::BadRequest, Provider::Custom)),
        429 => Some((Family::RateLimit, Provider::Custom)),
        503 => Some((Family::ServiceUnavailable, Provider::Custom)),
        504 => Some((Family::Timeout(504), Provider::Custom)),
        _ => None,
    }
}

#[must_use]
pub fn map_bedrock_exception(input: &BedrockExceptionInput) -> Option<ErrorClass> {
    let (family, provider) = mantle_classification(input)
        .or_else(|| classify_by_predicate(&input.error_str, input.status_code))
        .or_else(|| classify_by_status(input.status_code?))?;
    Some(build(input, family, provider))
}

fn build(input: &BedrockExceptionInput, family: Family, provider: Provider) -> ErrorClass {
    let provider_value = match provider {
        Provider::Literal => "bedrock".to_owned(),
        Provider::Custom => input.custom_llm_provider.clone(),
    };
    let context = ErrorContext {
        message: format!("{provider_value}: {}", reason(family)),
        model: Some(input.model.clone()),
        provider: Some(provider_value),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    match family {
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::PermissionDenied => ErrorClass::PermissionDenied(Box::new(context)),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
        Family::NotFound => ErrorClass::NotFound(Box::new(context)),
        Family::Timeout(original_status) => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(original_status),
        })),
    }
}
