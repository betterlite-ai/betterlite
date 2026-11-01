#![cfg(test)]

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use super::*;
use crate::error::details::{ApiDetails, GuardrailDetails};
use crate::facts::presence::Field;
use crate::facts::{ErrorCode, ProviderErrorBody};
use crate::parse::LitellmExceptionFacts;

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("built-in redaction patterns always compile")
}

fn raw(provider: &str, error_str: &str, status_code: Option<u16>) -> Box<RawException> {
    Box::new(RawException {
        model: "gpt-4".to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
    })
}

fn family(class: &ErrorClass) -> &'static str {
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

#[rstest]
#[case::already_mapped_identity_passes_through(
    DispatchInput::Mapped(ErrorClass::Authentication(Box::new(ErrorContext {
        message: "boom".to_owned(),
        model: Some("gpt-4".to_owned()),
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))),
    "authentication"
)]
#[case::guardrail_identity_passes_through(
    DispatchInput::Mapped(ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: Some("pii".to_owned()),
        message: "blocked".to_owned(),
        status_code: 400,
        blocked_content: true,
    }))),
    "guardrail_raised"
)]
#[case::timeout_preamble_outranks_status_code(
    DispatchInput::Raw(raw("openai", "Request timed out while calling upstream", Some(500))),
    "timeout"
)]
#[case::litellm_proxy_regex_match_raises_the_named_exception(
    DispatchInput::Raw(raw(
        "litellm_proxy",
        "litellm.ContextWindowExceededError: too many tokens",
        None,
    )),
    "context_window_exceeded"
)]
#[case::litellm_proxy_no_match_falls_through_to_openai_family(
    DispatchInput::Raw(raw("litellm_proxy", "Rate limit reached for requests", Some(400))),
    "rate_limit"
)]
#[case::openai_family_delegates_to_map_openai_exception(
    DispatchInput::Raw(raw("groq", "boom", Some(401))),
    "authentication"
)]
#[case::unknown_provider_reaches_the_unconditional_catchall(
    DispatchInput::Raw(raw("some-unknown-provider", "boom", None)),
    "api_connection"
)]
fn openai_errors_openai_error_tests_behavior(
    #[case] input: DispatchInput,
    #[case] expected_family: &str,
) {
    let class = dispatch(&input, &redactor());
    assert_eq!(family(&class), expected_family, "produced {class:?}");
}

#[test]
fn litellm_proxy_unconstructible_name_short_circuits_to_the_empty_identity_fallback() {
    let input = DispatchInput::Raw(raw(
        "litellm_proxy",
        "litellm.APIError: a generic failure",
        None,
    ));
    let ErrorClass::ApiConnection(context) = dispatch(&input, &redactor()) else {
        panic!("expected ApiConnection");
    };
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some(""));
}

#[test]
fn the_unconditional_catchall_keeps_the_real_model_and_provider_identity() {
    let input = DispatchInput::Raw(raw("some-unknown-provider", "boom", None));
    let ErrorClass::ApiConnection(context) = dispatch(&input, &redactor()) else {
        panic!("expected ApiConnection");
    };
    assert_eq!(context.model.as_deref(), Some("gpt-4"));
    assert_eq!(context.provider.as_deref(), Some("some-unknown-provider"));
}

#[test]
fn a_synthetic_response_status_never_overrides_the_timeout_preamble() {
    let input = DispatchInput::Raw(raw("openai", "The read operation timed out", Some(504)));
    let ErrorClass::Timeout(details) = dispatch(&input, &redactor()) else {
        panic!("expected Timeout");
    };
    assert_eq!(
        details.original_status.0, 408,
        "the preamble always reports 408, unlike the status-table's 504 branch"
    );
}

#[test]
fn malformed_litellm_proxy_body_falls_through_instead_of_panicking() {
    let mut request = raw(
        "litellm_proxy",
        "litellm.ContextWindowExceededError: too many tokens",
        None,
    );
    request.body = Some(OpenaiErrorBody {
        kind: None,
        param: None,
        code: Some("not actually json {{{".to_owned()),
        message: None,
        error: None,
    });
    let class = dispatch(&DispatchInput::Raw(request), &redactor());
    assert_eq!(family(&class), "context_window_exceeded");
}

#[test]
fn a_secret_looking_body_message_is_redacted_before_it_reaches_the_client() {
    let mut request = raw("openai", "boom", Some(401));
    request.body = Some(OpenaiErrorBody {
        kind: None,
        param: None,
        code: None,
        message: Some("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD".to_owned()),
        error: None,
    });
    let ErrorClass::Authentication(context) = dispatch(&DispatchInput::Raw(request), &redactor())
    else {
        panic!("expected Authentication");
    };
    assert!(
        !context
            .message
            .contains("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
        "message leaked an api-key-shaped secret: {}",
        context.message
    );
}

#[test]
fn rate_limit_classifications_never_carry_vendor_response_headers() {
    let mapped = dispatch(
        &DispatchInput::Raw(raw("openai", "Rate limit reached for requests", Some(429))),
        &redactor(),
    );
    let ErrorClass::RateLimit(details) = mapped else {
        panic!("expected RateLimit");
    };
    assert!(
        details.headers.is_empty(),
        "no branch this task owns ever forwards a vendor Set-Cookie/CORS/Authorization header"
    );
}

#[test]
fn mistral_streaming_still_classifies_as_api_through_the_openai_family_branch() {
    let input = DispatchInput::Raw(raw("mistral", "Mistral API raised a streaming error", None));
    let ErrorClass::Api(details) = dispatch(&input, &redactor()) else {
        panic!("expected Api");
    };
    assert_eq!(details.status, 500);
}

#[test]
fn exception_provider_label_matches_pythons_title_case_plus_exception_suffix() {
    assert_eq!(exception_provider_label("anthropic"), "AnthropicException");
    assert_eq!(exception_provider_label(""), "Exception");
}

#[test]
fn is_openai_family_recognizes_every_alias_and_every_compatible_provider() {
    assert!(is_openai_family("openai"));
    assert!(is_openai_family("text-completion-openai"));
    assert!(is_openai_family("custom_openai"));
    assert!(is_openai_family("mistral"));
    assert!(is_openai_family("runwayml"));
    assert!(is_openai_family("litellm_proxy"));
    assert!(is_openai_family("groq"));
    assert!(!is_openai_family("anthropic"));
}

#[rstest]
#[case::authentication_error("AuthenticationError", "authentication")]
#[case::bad_gateway_error("BadGatewayError", "bad_gateway")]
#[case::context_window_exceeded_error("ContextWindowExceededError", "context_window_exceeded")]
#[case::not_found_error("NotFoundError", "not_found")]
#[case::permission_denied_error("PermissionDeniedError", "permission_denied")]
#[case::service_unavailable_error("ServiceUnavailableError", "service_unavailable")]
#[case::unprocessable_entity_error("UnprocessableEntityError", "unprocessable_entity")]
#[case::unsupported_params_error("UnsupportedParamsError", "unsupported_params")]
#[case::content_policy_violation_error("ContentPolicyViolationError", "content_policy_violation")]
#[case::internal_server_error("InternalServerError", "internal_server")]
#[case::rate_limit_error("RateLimitError", "rate_limit")]
#[case::api_connection_error("APIConnectionError", "api_connection")]
#[case::api_response_validation_error("APIResponseValidationError", "api_response_validation")]
#[case::bad_request_error("BadRequestError", "bad_request")]
#[case::image_fetch_error("ImageFetchError", "bad_request")]
#[case::invalid_request_error("InvalidRequestError", "bad_request")]
fn classify_facts_maps_every_litellm_proxy_exception_name_to_its_family(
    #[case] name: &str,
    #[case] expected_family: &str,
) {
    let facts = LitellmExceptionFacts {
        name: name.to_owned(),
        message: "boom".to_owned(),
        model: "gpt-4".to_owned(),
        provider: "litellm_proxy".to_owned(),
        response: None,
        body: None,
    };
    assert_eq!(
        family(&litellm_proxy::classify_facts(&facts)),
        expected_family
    );
}

#[test]
fn classify_facts_reads_a_numeric_body_code_as_its_decimal_string() {
    let facts = LitellmExceptionFacts {
        name: "RateLimitError".to_owned(),
        message: "boom".to_owned(),
        model: "gpt-4".to_owned(),
        provider: "litellm_proxy".to_owned(),
        response: None,
        body: Some(ProviderErrorBody {
            message: Field::Missing,
            error: Field::Missing,
            code: Field::Value(ErrorCode::Number(42)),
        }),
    };
    let ErrorClass::RateLimit(details) = litellm_proxy::classify_facts(&facts) else {
        panic!("expected RateLimit");
    };
    assert_eq!(details.context.body_code.as_deref(), Some("42"));
}

#[test]
fn classify_facts_reads_a_text_body_code_verbatim() {
    let facts = LitellmExceptionFacts {
        name: "RateLimitError".to_owned(),
        message: "boom".to_owned(),
        model: "gpt-4".to_owned(),
        provider: "litellm_proxy".to_owned(),
        response: None,
        body: Some(ProviderErrorBody {
            message: Field::Missing,
            error: Field::Missing,
            code: Field::Value(ErrorCode::Text("rate_limit_exceeded".to_owned())),
        }),
    };
    let ErrorClass::RateLimit(details) = litellm_proxy::classify_facts(&facts) else {
        panic!("expected RateLimit");
    };
    assert_eq!(
        details.context.body_code.as_deref(),
        Some("rate_limit_exceeded")
    );
}

#[test]
fn classify_facts_leaves_body_code_absent_when_the_body_carries_no_code() {
    let facts = LitellmExceptionFacts {
        name: "RateLimitError".to_owned(),
        message: "boom".to_owned(),
        model: "gpt-4".to_owned(),
        provider: "litellm_proxy".to_owned(),
        response: None,
        body: Some(ProviderErrorBody {
            message: Field::Value("boom".to_owned()),
            error: Field::Missing,
            code: Field::Missing,
        }),
    };
    let ErrorClass::RateLimit(details) = litellm_proxy::classify_facts(&facts) else {
        panic!("expected RateLimit");
    };
    assert_eq!(details.context.body_code, None);
}

#[test]
fn the_litellm_proxy_branch_forwards_the_raw_bodys_code_into_the_raised_exception() {
    let mut request = raw(
        "litellm_proxy",
        "litellm.BadRequestError: invalid request",
        None,
    );
    request.body = Some(OpenaiErrorBody {
        kind: None,
        param: None,
        code: Some("content_filter".to_owned()),
        message: None,
        error: None,
    });
    let ErrorClass::BadRequest(context) = dispatch(&DispatchInput::Raw(request), &redactor())
    else {
        panic!("expected BadRequest");
    };
    assert_eq!(context.body_code.as_deref(), Some("content_filter"));
}

#[test]
fn an_unrecognized_facts_name_never_panics_and_classifies_as_unclassified() {
    let facts = LitellmExceptionFacts {
        name: "SomeFutureError".to_owned(),
        message: "boom".to_owned(),
        model: "gpt-4".to_owned(),
        provider: "litellm_proxy".to_owned(),
        response: None,
        body: None,
    };
    assert_eq!(
        family(&litellm_proxy::classify_facts(&facts)),
        "unclassified"
    );
}

#[test]
fn the_catchall_classifies_by_status_before_falling_back_to_api_connection() {
    let classified = dispatch(
        &DispatchInput::Raw(raw("some-unknown-provider", "boom", Some(429))),
        &redactor(),
    );
    let unclassified = dispatch(
        &DispatchInput::Raw(raw("some-unknown-provider", "boom", None)),
        &redactor(),
    );
    assert_eq!(family(&classified), "rate_limit");
    assert_eq!(family(&unclassified), "api_connection");
}

#[test]
fn a_synthesized_status_in_the_catchall_never_outranks_the_plain_api_connection_fallback() {
    let mut request = raw("some-unknown-provider", "boom", Some(429));
    request.status_code_is_synthesized = true;
    let class = dispatch(&DispatchInput::Raw(request), &redactor());
    assert_eq!(family(&class), "api_connection");
}

#[test]
fn api_details_status_is_forwarded_unchanged_through_mapped_passthrough() {
    let input = DispatchInput::Mapped(ErrorClass::Api(Box::new(ApiDetails {
        context: ErrorContext {
            message: "boom".to_owned(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        status: 418,
    })));
    let ErrorClass::Api(details) = dispatch(&input, &redactor()) else {
        panic!("expected Api");
    };
    assert_eq!(details.status, 418);
}
