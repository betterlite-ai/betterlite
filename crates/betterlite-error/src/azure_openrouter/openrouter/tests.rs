#![cfg(test)]

use rstest::rstest;

use super::*;

// mirrors ErrorClass's own #[serde(tag = "family", rename_all = "snake_case")] without touching serde_json::Value
fn family_of(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "authentication",
        ErrorClass::NotFound(_) => "not_found",
        ErrorClass::PermissionDenied(_) => "permission_denied",
        ErrorClass::BadRequest(_) => "bad_request",
        ErrorClass::UnprocessableEntity(_) => "unprocessable_entity",
        ErrorClass::Timeout(_) => "timeout",
        ErrorClass::RateLimit(_) => "rate_limit",
        ErrorClass::BudgetExceeded(_) => "budget_exceeded",
        ErrorClass::ContextWindowExceeded(_) => "context_window_exceeded",
        ErrorClass::ContentPolicyViolation(_) => "content_policy_violation",
        ErrorClass::ServiceUnavailable(_) => "service_unavailable",
        ErrorClass::BadGateway(_) => "bad_gateway",
        ErrorClass::InternalServer(_) => "internal_server",
        ErrorClass::Api(_) => "api",
        ErrorClass::ApiConnection(_) => "api_connection",
        ErrorClass::ApiResponseValidation(_) => "api_response_validation",
        ErrorClass::JsonSchemaValidation(_) => "json_schema_validation",
        ErrorClass::UnsupportedParams(_) => "unsupported_params",
        ErrorClass::GuardrailRaised(_) => "guardrail_raised",
        ErrorClass::MidStreamFallback(_) => "mid_stream_fallback",
        ErrorClass::Unclassified(_) => "unclassified",
    }
}

// every family status_classification can reach wraps an ErrorContext, directly or inside its details struct
fn message_of(class: &ErrorClass) -> &str {
    match class {
        ErrorClass::Authentication(context)
        | ErrorClass::NotFound(context)
        | ErrorClass::PermissionDenied(context)
        | ErrorClass::BadRequest(context)
        | ErrorClass::UnprocessableEntity(context)
        | ErrorClass::ContextWindowExceeded(context)
        | ErrorClass::ContentPolicyViolation(context)
        | ErrorClass::ServiceUnavailable(context)
        | ErrorClass::BadGateway(context)
        | ErrorClass::InternalServer(context)
        | ErrorClass::ApiConnection(context)
        | ErrorClass::ApiResponseValidation(context)
        | ErrorClass::UnsupportedParams(context)
        | ErrorClass::Unclassified(context) => &context.message,
        ErrorClass::Timeout(details) => &details.context.message,
        ErrorClass::RateLimit(details) => &details.context.message,
        ErrorClass::Api(details) => &details.context.message,
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_) => {
            panic!("openrouter classification never produces this family")
        }
    }
}

fn input(status_code: Option<u16>, status_code_is_synthesized: bool) -> OpenrouterExceptionInput {
    OpenrouterExceptionInput {
        model: "openrouter/some-model".to_owned(),
        custom_llm_provider: "openrouter".to_owned(),
        error_str: "boom".to_owned(),
        exception_provider: "OpenRouterException".to_owned(),
        status_code,
        status_code_is_synthesized,
    }
}

#[rstest]
#[case::status_400(input(Some(400), false), "bad_request")]
#[case::status_401(input(Some(401), false), "authentication")]
#[case::status_404(input(Some(404), false), "not_found")]
#[case::status_408(input(Some(408), false), "timeout")]
#[case::status_422(input(Some(422), false), "bad_request")]
#[case::status_429(input(Some(429), false), "rate_limit")]
#[case::status_503(input(Some(503), false), "service_unavailable")]
#[case::status_504(input(Some(504), false), "timeout")]
#[case::status_other(input(Some(418), false), "api")]
#[case::no_status_code_connection_fallback(input(None, false), "api_connection")]
#[case::synthesized_status_code_connection_fallback(input(Some(500), true), "api_connection")]
fn azure_openrouter_errors_behavior_tests_behavior(
    #[case] input: OpenrouterExceptionInput,
    #[case] expected_family: &str,
) {
    let class = map_openrouter_exception(&input);
    assert_eq!(
        family_of(&class),
        expected_family,
        "input {input:?} produced {class:?}"
    );
}

#[test]
fn no_502_branch_falls_back_to_api() {
    // unlike _map_azure_exception, _map_openrouter_exception has no explicit 502 branch
    let class = map_openrouter_exception(&input(Some(502), false));
    let ErrorClass::Api(details) = class else {
        panic!("expected Api, got {class:?}");
    };
    assert_eq!(details.status, 502);
}

#[test]
fn status_code_present_but_synthesized_still_routes_connection() {
    let class = map_openrouter_exception(&input(Some(429), true));
    assert!(matches!(class, ErrorClass::ApiConnection(_)));
}

#[rstest]
#[case::status_400_label(400, "OpenRouterException - boom")]
#[case::status_401_label(401, "AuthenticationError: OpenRouterException - boom")]
#[case::status_404_label(404, "NotFoundError: OpenRouterException - boom")]
#[case::status_408_label(408, "Timeout Error: OpenRouterException - boom")]
#[case::status_422_label(422, "BadRequestError: OpenRouterException - boom")]
#[case::status_429_label(429, "RateLimitError: OpenRouterException - boom")]
#[case::status_503_label(503, "ServiceUnavailableError: OpenRouterException - boom")]
#[case::status_504_label(504, "Timeout Error: OpenRouterException - boom")]
#[case::status_other_label(418, "APIError: OpenRouterException - boom")]
fn status_classification_renders_the_matching_label(#[case] status: u16, #[case] expected: &str) {
    let class = map_openrouter_exception(&input(Some(status), false));
    assert_eq!(message_of(&class), expected);
}

#[test]
fn no_status_code_message_names_the_exception_provider() {
    let class = map_openrouter_exception(&input(None, false));
    assert_eq!(
        message_of(&class),
        "APIConnectionError: OpenRouterException - boom"
    );
}
