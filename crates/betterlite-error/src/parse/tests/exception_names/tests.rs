#![cfg(test)]

use super::*;
use crate::parse::constructors::ExceptionName;

#[rstest]
#[case::valid(
    "prefix litellm.ContextWindowExceededError suffix",
    Some(ExceptionName::ContextWindowExceeded)
)]
#[case::missing_name("litellm.Error", None)]
#[case::missing_error_suffix("litellm.ContextWindowExceeded", None)]
#[case::not_litellm("provider error", None)]
#[case::unknown("litellm.UnknownProviderFailureError", None)]
#[case::later_match(
    "litellm.invalid litellm.BadRequestError",
    Some(ExceptionName::BadRequest)
)]
#[case::suffix("litellm.BadRequestErrorSuffix", Some(ExceptionName::BadRequest))]
#[case::keeps_first_exception_before_later_one(
    "litellm.RateLimitError details litellm.BadRequestError",
    Some(ExceptionName::RateLimit)
)]
#[case::does_not_split_underscore_in_name("litellm.RateLimitError_BadRequestError", None)]
fn error_facts_behavior_tests_behavior(
    #[case] text: &str,
    #[case] expected: Option<ExceptionName>,
) {
    assert_eq!(
        extract_exception(None, text, "model", "provider", None)
            .unwrap()
            .map(|facts| facts.name),
        expected
    );
}
