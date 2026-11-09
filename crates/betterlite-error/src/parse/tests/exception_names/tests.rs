#![cfg(test)]

use super::*;
use crate::parse::constructors::ExceptionName;

#[rstest]
#[case::valid(
    "prefix litellm.ContextWindowExceededError suffix",
    Some(ExceptionName::ContextWindowExceeded)
)]
#[case::authentication("litellm.AuthenticationError", Some(ExceptionName::Authentication))]
#[case::bad_gateway("litellm.BadGatewayError", Some(ExceptionName::BadGateway))]
#[case::not_found("litellm.NotFoundError", Some(ExceptionName::NotFound))]
#[case::permission_denied("litellm.PermissionDeniedError", Some(ExceptionName::PermissionDenied))]
#[case::service_unavailable(
    "litellm.ServiceUnavailableError",
    Some(ExceptionName::ServiceUnavailable)
)]
#[case::unprocessable_entity(
    "litellm.UnprocessableEntityError",
    Some(ExceptionName::UnprocessableEntity)
)]
#[case::unsupported_params(
    "litellm.UnsupportedParamsError",
    Some(ExceptionName::UnsupportedParams)
)]
#[case::content_policy_violation(
    "litellm.ContentPolicyViolationError",
    Some(ExceptionName::ContentPolicyViolation)
)]
#[case::image_fetch("litellm.ImageFetchError", Some(ExceptionName::ImageFetch))]
#[case::internal_server("litellm.InternalServerError", Some(ExceptionName::InternalServer))]
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
