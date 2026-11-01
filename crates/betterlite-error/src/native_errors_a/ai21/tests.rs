#![cfg(test)]

use rstest::rstest;

use super::*;

fn describe(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::Api(_) => "api",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => panic!("ai21 classification never produces this family"),
    }
}

fn facts(message: Option<&str>, status_code: Option<u16>) -> ProviderErrorFacts {
    ProviderErrorFacts {
        status_code,
        status_code_is_synthesized: false,
        message: message.map(str::to_owned),
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    }
}

#[rstest]
#[case::too_many_tokens(
    Some("Prompt has too many tokens for this model"),
    None,
    "context_window_exceeded"
)]
#[case::bad_or_missing_token(Some("Bad or missing API token."), None, "bad_request")]
#[case::status_401(None, Some(401), "authentication")]
#[case::status_408(None, Some(408), "timeout")]
#[case::status_422(None, Some(422), "bad_request")]
#[case::status_429(None, Some(429), "rate_limit")]
#[case::status_500_falls_to_api(None, Some(500), "api")]
fn native_errors_a_behavior_tests_behavior(
    #[case] message: Option<&str>,
    #[case] status_code: Option<u16>,
    #[case] expected: &str,
) {
    let class =
        map_ai21_exception(&facts(message, status_code), None).expect("expected a classification");
    assert_eq!(describe(&class), expected);
}

#[test]
fn message_absent_and_status_absent_returns_none() {
    assert!(map_ai21_exception(&facts(None, None), None).is_none());
}

#[test]
fn message_present_but_unmatched_falls_through_to_status() {
    let class = map_ai21_exception(
        &facts(Some("a completely unrelated message"), Some(429)),
        None,
    )
    .expect("expected a classification");
    assert_eq!(describe(&class), "rate_limit");
}

#[test]
fn message_predicate_wins_over_a_competing_status_code() {
    let class = map_ai21_exception(&facts(Some("Prompt has too many tokens"), Some(429)), None)
        .expect("expected a classification");
    assert_eq!(describe(&class), "context_window_exceeded");
}

#[test]
fn message_present_and_unmatched_and_status_absent_returns_none() {
    assert!(
        map_ai21_exception(&facts(Some("a completely unrelated message"), None), None).is_none()
    );
}

#[test]
fn api_fallback_carries_the_original_status() {
    let class =
        map_ai21_exception(&facts(None, Some(500)), None).expect("expected a classification");
    let ErrorClass::Api(details) = class else {
        panic!("expected Api, got {class:?}");
    };
    assert_eq!(details.status, 500);
}

#[test]
fn classified_branch_carries_model_and_provider() {
    let class = map_ai21_exception(&facts(None, Some(401)), Some("ai21/model-a".to_owned()))
        .expect("expected a classification");
    let ErrorClass::Authentication(context) = class else {
        panic!("expected Authentication, got {class:?}");
    };
    assert_eq!(context.model.as_deref(), Some("ai21/model-a"));
    assert_eq!(context.provider.as_deref(), Some("ai21"));
}
