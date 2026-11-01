#![cfg(test)]

use rstest::rstest;

use super::*;

fn describe(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::InternalServer(_) => "internal_server",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::Unclassified(_) => "unclassified",
        ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_) => {
            panic!("cohere classification never produces this family")
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
#[case::invalid_api_token("invalid api token", "", None, "authentication")]
#[case::missing_api_key("No API key provided.", "", None, "authentication")]
#[case::invalid_type_parameter("invalid type: parameter 'model'", "", None, "bad_request")]
#[case::too_many_tokens("too many tokens in prompt", "", None, "context_window_exceeded")]
#[case::internal_server_error_lowercase("internal server error", "", None, "internal_server")]
#[case::internal_server_error_mixed_case("Internal Server Error", "", None, "internal_server")]
#[case::status_400("unrecognized failure", "", Some(400), "bad_request")]
#[case::status_498("unrecognized failure", "", Some(498), "bad_request")]
#[case::status_408("unrecognized failure", "", Some(408), "timeout")]
#[case::status_500("unrecognized failure", "", Some(500), "internal_server")]
#[case::cohere_connection_error("connection reset", "CohereConnectionError", None, "rate_limit")]
#[case::invalid_type_generic("invalid type: of request", "SomeOtherError", None, "bad_request")]
#[case::unexpected_server_error(
    "Unexpected server error occurred",
    "SomeOtherError",
    None,
    "internal_server"
)]
fn native_errors_a_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] exception_type: &str,
    #[case] status_code: Option<u16>,
    #[case] expected: &str,
) {
    let class = map_cohere_exception(&facts(status_code), error_str, exception_type, None)
        .expect("expected a classification");
    assert_eq!(describe(&class), expected);
}

#[test]
fn status_present_but_unmatched_returns_none_even_with_matching_text_later() {
    // the matching text predicate below never runs: the hasattr(status_code) elif already resolved to NoMatch
    let outcome = map_cohere_exception(
        &facts(Some(403)),
        "Unexpected server error occurred",
        "SomeOtherError",
        None,
    );
    assert!(outcome.is_none());
}

#[test]
fn text_predicate_wins_over_status_code() {
    let class = map_cohere_exception(&facts(Some(403)), "invalid api token", "", None)
        .expect("expected a classification");
    assert_eq!(describe(&class), "authentication");
}

#[test]
fn totally_unmatched_input_is_unclassified() {
    let class = map_cohere_exception(
        &facts(None),
        "a completely unrelated failure",
        "SomeOtherError",
        None,
    )
    .expect("expected a classification");
    assert_eq!(describe(&class), "unclassified");
}

#[test]
fn unclassified_carries_the_raw_message_and_no_identity() {
    let class = map_cohere_exception(
        &facts(None),
        "a completely unrelated failure",
        "SomeOtherError",
        None,
    )
    .expect("expected a classification");
    let ErrorClass::Unclassified(context) = class else {
        panic!("expected Unclassified, got {class:?}");
    };
    assert_eq!(context.message, "a completely unrelated failure");
    assert_eq!(context.model, None);
    assert_eq!(context.provider, None);
}

#[test]
fn classified_branch_carries_model_and_provider() {
    let class = map_cohere_exception(
        &facts(None),
        "too many tokens",
        "",
        Some("command-r".to_owned()),
    )
    .expect("expected a classification");
    let ErrorClass::ContextWindowExceeded(context) = class else {
        panic!("expected ContextWindowExceeded, got {class:?}");
    };
    assert_eq!(context.model.as_deref(), Some("command-r"));
    assert_eq!(context.provider.as_deref(), Some("cohere"));
}
