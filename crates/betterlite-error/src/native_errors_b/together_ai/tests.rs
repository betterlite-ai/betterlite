#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(error_str: &str, status_code: Option<u16>) -> TogetherAiExceptionInput {
    TogetherAiExceptionInput {
        model: "together_ai/some-model".to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        message: None,
    }
}

fn family(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::BudgetExceeded(_)
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
            panic!("together_ai classification never produces {class:?}")
        }
    }
}

#[rstest]
// error_str predicates, lines 1730-1767
#[case::context_window(
    input(
        r#"{"error": "`inputs` tokens + `max_new_tokens` must be <= 4096"}"#,
        None
    ),
    Some("context_window_exceeded")
)]
#[case::invalid_private_key(
    input(r#"{"error": "invalid private key supplied"}"#, None),
    Some("authentication")
)]
#[case::invalid_argument(
    input(r#"{"error": "INVALID_ARGUMENT: bad request"}"#, None),
    Some("bad_request")
)]
#[case::timeout_string(input("A timeout occurred while waiting", None), Some("timeout"))]
#[case::api_key_format(
    input(r#"{"error": "API key doesn't match expected format."}"#, None),
    Some("bad_request")
)]
#[case::error_type_validation(
    input(r#"{"error_type": "validation", "error": "bad"}"#, None),
    Some("bad_request")
)]
// status_code fallback, lines 1769-1803
#[case::status_408(input("boom", Some(408)), Some("timeout"))]
#[case::status_422(input("boom", Some(422)), Some("bad_request"))]
#[case::status_429(input("boom", Some(429)), Some("rate_limit"))]
#[case::status_524(input("boom", Some(524)), Some("timeout"))]
#[case::status_present_but_unmatched_falls_through(input("boom", Some(418)), None)]
#[case::no_status_code_falls_through(input("boom", None), None)]
fn native_errors_b_behavior_tests_behavior(
    #[case] input: TogetherAiExceptionInput,
    #[case] expected_family: Option<&str>,
) {
    let mapped = map_together_ai_exception(&input);
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
    // the predicate chain at line 1730 runs before the status chain at line 1769
    let class = map_together_ai_exception(&input(
        r#"{"error": "invalid private key supplied"}"#,
        Some(429),
    ));
    assert_eq!(
        family(&class.expect("predicate always raises")),
        "authentication"
    );
}

#[test]
fn timeout_default_ignores_the_original_524_status() {
    // Timeout() never receives exception_status_code in the 524 branch, so it defaults to 408
    let class = map_together_ai_exception(&input("boom", Some(524)));
    let ErrorClass::Timeout(details) = class.expect("status 524 always raises") else {
        panic!("expected Timeout");
    };
    assert_eq!(details.original_status.0, 408);
}

#[test]
fn missing_message_falls_back_to_error_str_instead_of_panicking() {
    // every status branch reads original_exception.message unconditionally and crashes without it in LiteLLM; betterlite falls back to error_str instead (docs/deviations/B242.yaml)
    let class = map_together_ai_exception(&input("boom", Some(429)));
    let ErrorClass::RateLimit(details) = class.expect("status 429 always raises") else {
        panic!("expected RateLimit");
    };
    assert_eq!(details.context.message, "together_ai: boom");
}

#[test]
fn missing_status_code_never_panics_and_falls_through() {
    // a missing status_code attribute crashes with AttributeError in LiteLLM; betterlite falls through gracefully instead (docs/deviations/B242.yaml)
    let class = map_together_ai_exception(&input("boom", None));
    assert!(class.is_none());
}

#[test]
fn error_response_without_an_error_key_still_classifies_status_422() {
    // line 1778 subscripts error_response['error'] unconditionally and would KeyError in LiteLLM without it; betterlite never needs the key to classify the status
    let class = map_together_ai_exception(&input(r#"{"message": "bad request"}"#, Some(422)));
    assert_eq!(
        family(&class.expect("status 422 always raises")),
        "bad_request"
    );
}
