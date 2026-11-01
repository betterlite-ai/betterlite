#![cfg(test)]

use rstest::rstest;

use super::*;

fn describe(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::RateLimit(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContextWindowExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => {
            panic!("cloudflare classification never produces this family")
        }
    }
}

#[rstest]
#[case::authentication_error("Authentication error: invalid token", "authentication")]
#[case::missing_required_property("payload must have required property 'model'", "bad_request")]
#[case::authentication_checked_before_required_property(
    "Authentication error and payload must have required property 'model'",
    "authentication"
)]
fn native_errors_a_behavior_tests_behavior(#[case] error_str: &str, #[case] expected: &str) {
    let class = map_cloudflare_exception(error_str, Some("cloudflare/model-a".to_owned()))
        .expect("expected a classification");
    assert_eq!(describe(&class), expected);
}

#[test]
fn unmatched_error_returns_none() {
    assert!(
        map_cloudflare_exception("some other failure", Some("cloudflare/model-a".to_owned()))
            .is_none()
    );
}

#[test]
fn matched_authentication_sets_model_and_provider() {
    let class = map_cloudflare_exception(
        "Authentication error",
        Some("cloudflare/model-a".to_owned()),
    )
    .expect("expected a classification");
    let ErrorClass::Authentication(context) = class else {
        panic!("expected Authentication, got {class:?}");
    };
    assert_eq!(context.model.as_deref(), Some("cloudflare/model-a"));
    assert_eq!(context.provider.as_deref(), Some("cloudflare"));
}

#[test]
fn matched_bad_request_tolerates_missing_model() {
    let class = map_cloudflare_exception("must have required property 'model'", None)
        .expect("expected a classification");
    let ErrorClass::BadRequest(context) = class else {
        panic!("expected BadRequest, got {class:?}");
    };
    assert_eq!(context.model, None);
}
