use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::{ExceptionCheckers, ProviderErrorFacts};
use crate::rate_limit::RateLimitCategory;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    BadRequest,
    ContextWindowExceeded,
    InternalServer,
    Authentication,
    ContentPolicyViolation,
    RateLimit,
    PermissionDenied,
    NotFound,
    Timeout,
    ApiConnection,
    ServiceUnavailable,
}

type PhraseRule = (&'static [&'static str], Family, &'static str);

// branch-1153 and branch-1167, tried before the shared context-window checker
const EARLY_PHRASE_RULES: &[PhraseRule] = &[
    (
        &[
            "Vertex AI API has not been used in project",
            "Unable to find your project",
        ],
        Family::BadRequest,
        "the Vertex AI project could not be resolved",
    ),
    (
        &["400 Request payload size exceeds"],
        Family::ContextWindowExceeded,
        "the request payload exceeds Vertex AI's size limit",
    ),
];

// branch-1180, branch-1192 and branch-1199, tried right after the shared context-window checker
const MID_PHRASE_RULES: &[PhraseRule] = &[
    (
        &["None Unknown Error.", "Content has no parts."],
        Family::InternalServer,
        "Vertex AI returned an empty or unparsable response",
    ),
    (
        &["API key not valid."],
        Family::Authentication,
        "the Vertex AI API key is not valid",
    ),
    (
        &["403"],
        Family::BadRequest,
        "Vertex AI rejected the request",
    ),
];

// branch-1213, then branch-1230's five quota-or-capacity phrases
const LATE_PHRASE_RULES: &[PhraseRule] = &[
    (
        &[
            "The response was blocked.",
            "Output blocked by content filtering policy",
        ],
        Family::ContentPolicyViolation,
        "Vertex AI blocked the response",
    ),
    (
        &[
            "429 Quota exceeded",
            "Quota exceeded for",
            "Resource exhausted",
            "IndexError: list index out of range",
            "429 Unable to submit request because the service is temporarily out of capacity.",
        ],
        Family::RateLimit,
        "Vertex AI's quota for this request was exceeded",
    ),
];

// branch-1272, tried last, after the status-dependent envelope check in classify_envelope
const FINAL_PHRASE_RULES: &[PhraseRule] = &[(
    &["500 Internal Server Error", "The model is overloaded."],
    Family::InternalServer,
    "Vertex AI reported an internal server error",
)];

// the first rule in the slice whose phrases match wins, mirroring an elif chain over literal substrings
fn phrase_family(error_str: &str, rules: &[PhraseRule]) -> Option<(Family, &'static str)> {
    for (phrases, family, reason) in rules {
        if phrases.iter().any(|phrase| error_str.contains(phrase)) {
            return Some((*family, reason));
        }
    }
    None
}

// branch-1250: a 429 wrapped inside a 5xx envelope, LiteLLM's only predicate that also reads status_code
fn classify_envelope(error_str: &str, status_code: Option<u16>) -> Option<(Family, &'static str)> {
    let is_5xx_wrapped_429 = status_code.is_some_and(|status| (500..600).contains(&status))
        && crate::parse::get_body_error_code(error_str) == Some(429);
    is_5xx_wrapped_429.then_some((
        Family::RateLimit,
        "the gateway wrapped a 429 inside a 5xx envelope",
    ))
}

// the status_code fallback LiteLLM only reaches once every error_str predicate above has missed
const fn classify_by_status(status_code: u16) -> Option<(Family, &'static str)> {
    match status_code {
        400 => Some((Family::BadRequest, "Vertex AI rejected the request body")),
        401 => Some((
            Family::Authentication,
            "the Vertex AI credentials were rejected",
        )),
        403 => Some((
            Family::PermissionDenied,
            "Vertex AI denied permission for the request",
        )),
        404 => Some((
            Family::NotFound,
            "Vertex AI could not find the requested resource",
        )),
        408 => Some((Family::Timeout, "the request to Vertex AI timed out")),
        429 => Some((Family::RateLimit, "Vertex AI rate limited the request")),
        500 => Some((
            Family::InternalServer,
            "Vertex AI reported an internal server error",
        )),
        502 => Some((Family::ApiConnection, "the connection to Vertex AI failed")),
        503 => Some((
            Family::ServiceUnavailable,
            "Vertex AI is temporarily unavailable",
        )),
        _ => None,
    }
}

#[must_use]
pub fn map_vertex_exception(
    error: &ProviderErrorFacts,
    error_str: &str,
    custom_llm_provider: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    // mirrors _map_vertex_exception's elif chain: the error_str predicates run first and win over status_code
    let (family, reason) = phrase_family(error_str, EARLY_PHRASE_RULES)
        .or_else(|| {
            // branch-1173: the shared context-window checker other tasks' predicates already implement and reuse
            ExceptionCheckers::context_window(error_str).then_some((
                Family::ContextWindowExceeded,
                "the request exceeded the model's context window",
            ))
        })
        .or_else(|| phrase_family(error_str, MID_PHRASE_RULES))
        .or_else(|| phrase_family(error_str, LATE_PHRASE_RULES))
        .or_else(|| classify_envelope(error_str, error.status_code))
        .or_else(|| phrase_family(error_str, FINAL_PHRASE_RULES))
        .or_else(|| classify_by_status(error.status_code?))?;
    let context = ErrorContext {
        message: format!("{custom_llm_provider}: {reason}"),
        model,
        provider: Some(custom_llm_provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    Some(build_error_class(family, context))
}

fn build_error_class(family: Family, context: ErrorContext) -> ErrorClass {
    match family {
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::ContentPolicyViolation => ErrorClass::ContentPolicyViolation(Box::new(context)),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        Family::PermissionDenied => ErrorClass::PermissionDenied(Box::new(context)),
        Family::NotFound => ErrorClass::NotFound(Box::new(context)),
        Family::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus::default(),
        })),
        Family::ApiConnection => ErrorClass::ApiConnection(Box::new(context)),
        Family::ServiceUnavailable => ErrorClass::ServiceUnavailable(Box::new(context)),
    }
}
