use std::collections::BTreeMap;

use crate::error::details::{RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::ProviderErrorFacts;
use crate::rate_limit::RateLimitCategory;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Authentication,
    BadRequest,
    ContextWindowExceeded,
    InternalServer,
    Timeout,
    RateLimit,
}

// an unmatched status_code resolves the elif chain into NoMatch; a total miss falls to the dead-code-free Unclassified
enum Outcome {
    Classified(Family, &'static str),
    NoMatch,
    Unclassified,
}

// the four elif branches checked before status_code is ever consulted
fn classify_by_text(error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("invalid api token") || error_str.contains("No API key provided.") {
        return Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        ));
    }
    if error_str.contains("invalid type: parameter") {
        return Some((
            Family::BadRequest,
            "the request parameter has an invalid type",
        ));
    }
    if error_str.contains("too many tokens") {
        return Some((
            Family::ContextWindowExceeded,
            "the request exceeded the model's context window",
        ));
    }
    if error_str.to_lowercase().contains("internal server error") {
        return Some((
            Family::InternalServer,
            "the provider reported an internal server error",
        ));
    }
    None
}

// the three elif branches checked only once hasattr(status_code) and classify_by_text both missed
fn classify_by_tail(error_str: &str, exception_type: &str) -> Option<(Family, &'static str)> {
    if exception_type.contains("CohereConnectionError") {
        return Some((
            Family::RateLimit,
            "repeated connection errors were treated as a rate limit",
        ));
    }
    if error_str.contains("invalid type:") {
        return Some((
            Family::BadRequest,
            "the request parameter has an invalid type",
        ));
    }
    if error_str.contains("Unexpected server error") {
        return Some((
            Family::InternalServer,
            "the provider reported an internal server error",
        ));
    }
    None
}

fn classify(facts: &ProviderErrorFacts, error_str: &str, exception_type: &str) -> Outcome {
    if let Some((family, reason)) = classify_by_text(error_str) {
        return Outcome::Classified(family, reason);
    }
    if let Some(status) = facts.status_code {
        // this elif branch is the only one consulted once status_code is present; an unmatched status falls to NoMatch, never to classify_by_tail
        return match status {
            400 | 498 => {
                Outcome::Classified(Family::BadRequest, "the provider rejected the request")
            }
            408 => Outcome::Classified(Family::Timeout, "the request to the provider timed out"),
            500 => Outcome::Classified(
                Family::InternalServer,
                "the provider reported an internal server error",
            ),
            _ => Outcome::NoMatch,
        };
    }
    classify_by_tail(error_str, exception_type).map_or(Outcome::Unclassified, |(family, reason)| {
        Outcome::Classified(family, reason)
    })
}

fn build(family: Family, context: ErrorContext) -> ErrorClass {
    match family {
        Family::Authentication => ErrorClass::Authentication(Box::new(context)),
        Family::BadRequest => ErrorClass::BadRequest(Box::new(context)),
        Family::ContextWindowExceeded => ErrorClass::ContextWindowExceeded(Box::new(context)),
        Family::InternalServer => ErrorClass::InternalServer(Box::new(context)),
        Family::Timeout => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        Family::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
    }
}

#[must_use]
pub fn map_cohere_exception(
    facts: &ProviderErrorFacts,
    error_str: &str,
    exception_type: &str,
    model: Option<String>,
) -> Option<ErrorClass> {
    match classify(facts, error_str, exception_type) {
        Outcome::NoMatch => None,
        // re-raises the exception unchanged, so unlike every classified branch it carries no model/provider identity
        Outcome::Unclassified => Some(ErrorClass::Unclassified(Box::new(ErrorContext {
            message: error_str.to_owned(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        }))),
        Outcome::Classified(family, reason) => Some(build(
            family,
            ErrorContext {
                message: reason.to_owned(),
                model,
                provider: Some("cohere".to_owned()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            },
        )),
    }
}
