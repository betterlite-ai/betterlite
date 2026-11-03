#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(error_str: &str, status_code: Option<u16>) -> AlephAlphaExceptionInput {
    AlephAlphaExceptionInput {
        model: "aleph_alpha/some-model".to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        message: None,
    }
}

fn family(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::ServiceUnavailable(_) => "service_unavailable",
        ErrorClass::ApiConnection(_) => "api_connection",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => {
            panic!("aleph_alpha classification never produces {class:?}")
        }
    }
}

#[rstest]
// error_str predicates, lines 1816-1829
#[case::context_window(
    input("This is longer than the model's maximum context length", None),
    Some("context_window_exceeded")
)]
#[case::context_window_before_invalid_token(
    input(
        "This is longer than the model's maximum context length; InvalidToken",
        None
    ),
    Some("context_window_exceeded")
)]
#[case::invalid_token(input("InvalidToken supplied", None), Some("bad_request"))]
#[case::no_token_provided(input("No token provided", None), Some("bad_request"))]
// status fallback, lines 1830-1860
#[case::status_401(input("boom", Some(401)), Some("authentication"))]
#[case::status_400(input("boom", Some(400)), Some("bad_request"))]
#[case::status_429(input("boom", Some(429)), Some("rate_limit"))]
#[case::status_500(input("boom", Some(500)), Some("service_unavailable"))]
#[case::status_present_but_unmatched_reraises(input("boom", Some(418)), Some("api_connection"))]
#[case::no_status_code_reraises(input("boom", None), Some("api_connection"))]
fn native_errors_b_behavior_tests_behavior(
    #[case] input: AlephAlphaExceptionInput,
    #[case] expected_family: Option<&str>,
) {
    let mapped = map_aleph_alpha_exception(&input);
    match expected_family {
        None => assert!(
            mapped.is_none(),
            "expected no classification, got {mapped:?}"
        ),
        Some(expected) => {
            let class = mapped.expect("a matching predicate must produce a class");
            assert_eq!(family(&class), expected);
        }
    }
}

#[test]
fn error_str_predicate_wins_over_a_competing_status_code() {
    // line 1816 and 1823 both run before the hasattr(status_code) chain at line 1830
    let class = map_aleph_alpha_exception(&input("InvalidToken supplied", Some(500)));
    assert_eq!(
        family(&class.expect("predicate always raises")),
        "bad_request"
    );
}

#[test]
fn missing_message_falls_back_to_error_str_instead_of_panicking() {
    // every status branch reads original_exception.message unconditionally and crashes without it in LiteLLM; betterlite falls back to error_str instead (docs/deviations/B242.yaml)
    let class = map_aleph_alpha_exception(&input("boom", Some(401)));
    let ErrorClass::Authentication(details) = class.expect("status 401 always raises") else {
        panic!("expected Authentication");
    };
    assert_eq!(details.message, "aleph_alpha: boom");
}

#[rstest]
#[case::context_window_on_500("This is longer than the model's maximum context length", 500)]
#[case::bad_request_status_400("boom", 400)]
fn response_branches_answer_the_provider_status(#[case] error_str: &str, #[case] status: u16) {
    let class = map_aleph_alpha_exception(&input(error_str, Some(status)))
        .expect("expected a classification");
    assert_eq!(class.metadata().status_code, status);
}

#[test]
fn status_401_keeps_the_class_default() {
    let class =
        map_aleph_alpha_exception(&input("boom", Some(401))).expect("expected a classification");
    assert_eq!(class.metadata().status_code, 401);
}

#[test]
fn unmatched_and_missing_status_answer_the_outer_api_connection() {
    let class = map_aleph_alpha_exception(&input("boom", Some(404)))
        .expect("unmatched status is handled by the outer connection error");
    let metadata = class.metadata();
    let ErrorClass::ApiConnection(details) = class else {
        panic!("expected API connection error");
    };
    assert_eq!(metadata.status_code, 500);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(""));
    assert_eq!(metadata.error_type, crate::envelope::WireField::Null);
}
