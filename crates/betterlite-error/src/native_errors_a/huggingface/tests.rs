#![cfg(test)]

use rstest::rstest;

use super::*;

fn describe(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::ServiceUnavailable(_) => "service_unavailable",
        ErrorClass::Api(_) => "api",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => {
            panic!("huggingface classification never produces this family")
        }
    }
}

fn facts(status_code: Option<u16>) -> ProviderErrorFacts {
    ProviderErrorFacts {
        status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    }
}

#[rstest]
#[case::length_limit_exceeded(
    "length limit exceeded for this model",
    None,
    "context_window_exceeded"
)]
#[case::invalid_user_token("A valid user token is required", None, "bad_request")]
#[case::rate_limit_reached("Rate limit reached for this endpoint", None, "rate_limit")]
#[case::status_401("unrecognized failure", Some(401), "authentication")]
#[case::status_400("unrecognized failure", Some(400), "bad_request")]
#[case::status_408("unrecognized failure", Some(408), "timeout")]
#[case::status_429("unrecognized failure", Some(429), "rate_limit")]
#[case::status_503("unrecognized failure", Some(503), "service_unavailable")]
#[case::status_418_falls_to_api("unrecognized failure", Some(418), "api")]
fn native_errors_a_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected: &str,
) {
    let class = map_huggingface_exception(&facts(status_code), error_str, None)
        .expect("expected a classification");
    assert_eq!(describe(&class), expected);
}

#[test]
fn unmatched_text_and_missing_status_returns_none() {
    assert!(
        map_huggingface_exception(&facts(None), "a completely unrelated failure", None).is_none()
    );
}

#[test]
fn text_predicate_wins_even_when_status_code_is_present() {
    let class = map_huggingface_exception(&facts(Some(500)), "Rate limit reached", None)
        .expect("expected a classification");
    assert_eq!(describe(&class), "rate_limit");
}

#[test]
fn api_fallback_carries_the_original_status() {
    let class = map_huggingface_exception(&facts(Some(418)), "unrecognized failure", None)
        .expect("expected a classification");
    let ErrorClass::Api(details) = class else {
        panic!("expected Api, got {class:?}");
    };
    assert_eq!(details.status, 418);
}

#[test]
fn classified_branch_carries_model_and_provider() {
    let class = map_huggingface_exception(
        &facts(None),
        "length limit exceeded",
        Some("hf/model-a".to_owned()),
    )
    .expect("expected a classification");
    let ErrorClass::ContextWindowExceeded(context) = class else {
        panic!("expected ContextWindowExceeded, got {class:?}");
    };
    assert_eq!(context.model.as_deref(), Some("hf/model-a"));
    assert_eq!(context.provider.as_deref(), Some("huggingface"));
}
