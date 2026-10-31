#![cfg(test)]

use super::*;

#[rstest]
#[case::valid(
    "prefix litellm.ContextWindowExceededError suffix",
    Some("ContextWindowExceededError")
)]
#[case::missing_name("litellm.Error", None)]
#[case::missing_error_suffix("litellm.ContextWindowExceeded", None)]
#[case::not_litellm("provider error", None)]
#[case::unknown("litellm.UnknownProviderFailureError", None)]
#[case::later_match("litellm.invalid litellm.BadRequestError", Some("BadRequestError"))]
#[case::suffix("litellm.BadRequestErrorSuffix", Some("BadRequestError"))]
#[case::keeps_first_exception_before_later_one(
    "litellm.RateLimitError details litellm.BadRequestError",
    Some("RateLimitError")
)]
#[case::does_not_split_underscore_in_name("litellm.RateLimitError_BadRequestError", None)]
fn error_facts_behavior_tests_behavior(#[case] text: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        extract_exception(None, text, "model", "provider", None)
            .unwrap()
            .map(|facts| facts.name),
        expected.map(str::to_owned)
    );
}
