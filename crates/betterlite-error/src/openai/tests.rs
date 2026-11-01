#![cfg(test)]

use rstest::rstest;

use super::body::{OpenaiErrorBody, OpenaiErrorDetail};
use super::*;

fn describe(class: &ErrorClass) -> (&'static str, &str) {
    match class {
        ErrorClass::Authentication(context) => ("authentication", &context.message),
        ErrorClass::NotFound(context) => ("not_found", &context.message),
        ErrorClass::PermissionDenied(context) => ("permission_denied", &context.message),
        ErrorClass::BadRequest(context) => ("bad_request", &context.message),
        ErrorClass::UnprocessableEntity(context) => ("unprocessable_entity", &context.message),
        ErrorClass::ContextWindowExceeded(context) => ("context_window_exceeded", &context.message),
        ErrorClass::ContentPolicyViolation(context) => {
            ("content_policy_violation", &context.message)
        }
        ErrorClass::ServiceUnavailable(context) => ("service_unavailable", &context.message),
        ErrorClass::BadGateway(context) => ("bad_gateway", &context.message),
        ErrorClass::InternalServer(context) => ("internal_server", &context.message),
        ErrorClass::ApiConnection(context) => ("api_connection", &context.message),
        ErrorClass::ApiResponseValidation(context) => ("api_response_validation", &context.message),
        ErrorClass::UnsupportedParams(context) => ("unsupported_params", &context.message),
        ErrorClass::Unclassified(context) => ("unclassified", &context.message),
        ErrorClass::Timeout(details) => ("timeout", &details.context.message),
        ErrorClass::RateLimit(details) => ("rate_limit", &details.context.message),
        ErrorClass::Api(details) => ("api", &details.context.message),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_) => {
            panic!("openai classification never produces this family")
        }
    }
}

fn input(error_str: &str, status_code: Option<u16>) -> OpenaiExceptionInput {
    OpenaiExceptionInput {
        model: "gpt-4".to_owned(),
        custom_llm_provider: "openai".to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        message: None,
        body: None,
    }
}

#[rstest]
#[case::rate_limit_text(input("Rate limit reached for requests", Some(400)), "rate_limit")]
#[case::rate_limit_standalone_429_word(input("error code: 429", None), "rate_limit")]
#[case::context_window_text(
    input("This model's maximum context length is 4096 tokens", None),
    "context_window_exceeded"
)]
#[case::model_not_found_text(
    input("invalid_request_error: model_not_found - no such model", None),
    "not_found"
)]
#[case::timeout_string_text(input("A timeout occurred while waiting", None), "timeout")]
#[case::content_policy_invalid_request_text(
    input(
        "invalid_request_error raised: content_policy_violation detected",
        Some(400)
    ),
    "content_policy_violation"
)]
#[case::content_policy_invalid_prompt_text(
    input("Invalid prompt: this is violating our usage policy", None),
    "content_policy_violation"
)]
#[case::content_policy_safety_system_text(
    input("the Request was rejected as a result of the safety system", None),
    "content_policy_violation"
)]
#[case::invalid_encrypted_content_text(
    input("invalid_encrypted_content supplied in request", Some(400)),
    "bad_request"
)]
#[case::could_not_be_verified_text(
    input("the signature could not be verified", Some(400)),
    "bad_request"
)]
#[case::invalid_request_error_text(input("invalid_request_error: bad field", None), "bad_request")]
#[case::internal_server_unknown_error_text(
    input("Web server is returning an unknown error", None),
    "internal_server"
)]
#[case::internal_server_processing_text(
    input("The server had an error processing your request.", None),
    "internal_server"
)]
#[case::request_too_large_text(input("Request too large for the model", None), "rate_limit")]
#[case::missing_credential_text(
    input(
        "The api_key client option must be set either by passing api_key to the client or by \
setting the OPENAI_API_KEY environment variable",
        None,
    ),
    "authentication"
)]
#[case::mistral_streaming_text(input("Mistral API raised a streaming error", None), "api")]
#[case::status_400(input("boom", Some(400)), "bad_request")]
#[case::status_401(input("boom", Some(401)), "authentication")]
#[case::status_404(input("boom", Some(404)), "not_found")]
#[case::status_408(input("boom", Some(408)), "timeout")]
#[case::status_422(input("boom", Some(422)), "bad_request")]
#[case::status_429(input("boom", Some(429)), "rate_limit")]
#[case::status_500(input("boom", Some(500)), "internal_server")]
#[case::status_502(input("boom", Some(502)), "bad_gateway")]
#[case::status_503(input("boom", Some(503)), "service_unavailable")]
#[case::status_504(input("boom", Some(504)), "timeout")]
#[case::status_other(input("boom", Some(418)), "api")]
#[case::no_status_connection_fallback(input("boom", None), "api_connection")]
fn openai_errors_openai_error_tests_behavior(
    #[case] input: OpenaiExceptionInput,
    #[case] expected_family: &str,
) {
    let class = map_openai_exception(&input);
    assert_eq!(
        describe(&class).0,
        expected_family,
        "input {input:?} produced {class:?}"
    );
}

#[test]
fn rate_limit_text_outranks_context_window_text() {
    let class = map_openai_exception(&input(
        "Rate limit reached: maximum context length is 4096",
        None,
    ));
    assert!(matches!(class, ErrorClass::RateLimit(_)));
}

#[test]
fn timeout_string_preamble_check_uses_a_different_phrase_than_the_branch_check() {
    // the preamble (dispatch.rs) checks "Request timed out"; this branch only recognizes "A timeout occurred"
    let class = map_openai_exception(&input("Request timed out waiting for a response", None));
    assert!(!matches!(class, ErrorClass::Timeout(_)));
}

#[test]
fn status_504_preserves_the_original_status_unlike_every_other_timeout_branch() {
    let ErrorClass::Timeout(details) = map_openai_exception(&input("boom", Some(504))) else {
        panic!("expected Timeout");
    };
    assert_eq!(details.original_status.0, 504);
}

#[test]
fn status_408_defaults_to_408_even_without_an_explicit_status_code_override() {
    let ErrorClass::Timeout(details) = map_openai_exception(&input("boom", Some(408))) else {
        panic!("expected Timeout");
    };
    assert_eq!(details.original_status.0, 408);
}

#[test]
fn timeout_string_branch_always_reports_status_408() {
    let ErrorClass::Timeout(details) =
        map_openai_exception(&input("A timeout occurred", Some(504)))
    else {
        panic!("expected Timeout");
    };
    assert_eq!(
        details.original_status.0, 408,
        "a synthetic response status must not override the timeout-string branch's own default"
    );
}

#[test]
fn rate_limit_classification_always_uses_vendor_rate_limit_category_and_no_dimension() {
    let ErrorClass::RateLimit(details) = map_openai_exception(&input("Rate limit reached", None))
    else {
        panic!("expected RateLimit");
    };
    assert_eq!(details.category, RateLimitCategory::VendorRateLimit);
    assert_eq!(details.dimension, None);
    assert!(details.headers.is_empty());
}

#[test]
fn status_400_forwards_body_type_param_code() {
    let mut request = input("boom", Some(400));
    request.body = Some(OpenaiErrorBody {
        kind: Some("invalid_request_error".to_owned()),
        param: Some("messages".to_owned()),
        code: Some("missing_field".to_owned()),
        message: None,
        error: None,
    });
    let ErrorClass::BadRequest(context) = map_openai_exception(&request) else {
        panic!("expected BadRequest");
    };
    assert_eq!(context.body_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(context.body_param.as_deref(), Some("messages"));
    assert_eq!(context.body_code.as_deref(), Some("missing_field"));
}

#[test]
fn status_500_forwards_body_param_and_code_but_not_a_literal_type() {
    let mut request = input("boom", Some(500));
    request.body = Some(OpenaiErrorBody {
        kind: Some("ignored".to_owned()),
        param: Some("messages".to_owned()),
        code: Some("oops".to_owned()),
        message: None,
        error: None,
    });
    let ErrorClass::InternalServer(context) = map_openai_exception(&request) else {
        panic!("expected InternalServer");
    };
    assert_eq!(context.body_param.as_deref(), Some("messages"));
    assert_eq!(context.body_code.as_deref(), Some("oops"));
}

#[test]
fn resolve_message_prefers_the_body_message_over_error_str() {
    let mut request = input("boom", Some(401));
    request.body = Some(OpenaiErrorBody {
        kind: None,
        param: None,
        code: None,
        message: Some("from the body".to_owned()),
        error: None,
    });
    let class = map_openai_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AuthenticationError: OpenAIException - from the body"
    );
}

#[test]
fn resolve_message_falls_back_to_nested_error_message() {
    let mut request = input("boom", Some(401));
    request.body = Some(OpenaiErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(OpenaiErrorDetail {
            message: Some("nested message".to_owned()),
        }),
    });
    let class = map_openai_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AuthenticationError: OpenAIException - nested message"
    );
}

#[test]
fn resolve_message_prefers_the_message_field_over_error_str() {
    let mut request = input("boom", Some(401));
    request.message = Some("the real message".to_owned());
    let class = map_openai_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AuthenticationError: OpenAIException - the real message"
    );
}

#[test]
fn exception_provider_is_recomputed_from_custom_llm_provider_not_from_the_caller() {
    let mut request = input("boom", Some(401));
    request.custom_llm_provider = "groq".to_owned();
    let class = map_openai_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AuthenticationError: GroqException - boom"
    );
}

#[test]
fn message_replaces_the_literal_openai_marker_with_the_real_provider_name() {
    let mut request = input(
        "an OPENAI error happened: openai.OpenAIError: boom",
        Some(401),
    );
    request.custom_llm_provider = "groq".to_owned();
    let class = map_openai_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AuthenticationError: GroqException - an GROQ error happened: groq.groqError: boom"
    );
}

#[test]
fn no_status_code_message_names_the_exception_provider() {
    let class = map_openai_exception(&input("boom", None));
    assert_eq!(
        describe(&class).1,
        "APIConnectionError: OpenAIException - boom"
    );
}

#[test]
fn content_policy_text_needs_both_invalid_prompt_and_the_usage_policy_phrase() {
    let class = map_openai_exception(&input("Invalid prompt: this one is fine", None));
    assert_eq!(describe(&class).0, "api_connection");
}

#[test]
fn content_policy_text_needs_the_invalid_prompt_prefix_too() {
    let class = map_openai_exception(&input(
        "this text is violating our usage policy somehow",
        None,
    ));
    assert_eq!(describe(&class).0, "api_connection");
}

#[test]
fn encrypted_content_text_alone_is_enough_without_the_could_not_be_verified_phrase() {
    let class = map_openai_exception(&input("invalid_encrypted_content supplied", None));
    assert_eq!(
        describe(&class).1,
        "OpenAIException - invalid_encrypted_content supplied\n\n\
Encrypted content affinity is required across deployments."
    );
}

#[test]
fn could_not_be_verified_text_alone_is_enough_without_the_encrypted_content_phrase() {
    let class = map_openai_exception(&input("the signature could not be verified", None));
    assert_eq!(
        describe(&class).1,
        "OpenAIException - the signature could not be verified\n\n\
Encrypted content affinity is required across deployments."
    );
}

#[test]
fn rate_limit_text_produces_the_rate_limit_error_message_prefix() {
    let class = map_openai_exception(&input("Rate limit reached for requests", None));
    assert_eq!(
        describe(&class).1,
        "RateLimitError: OpenAIException - Rate limit reached for requests"
    );
}

#[test]
fn context_window_text_does_not_forward_the_request_body() {
    let mut request = input("This model's maximum context length is 4096 tokens", None);
    request.body = Some(OpenaiErrorBody {
        kind: Some("invalid_request_error".to_owned()),
        param: Some("messages".to_owned()),
        code: Some("context_length".to_owned()),
        message: None,
        error: None,
    });
    let ErrorClass::ContextWindowExceeded(context) = map_openai_exception(&request) else {
        panic!("expected ContextWindowExceeded");
    };
    assert_eq!(context.body_type, None);
    assert_eq!(context.body_param, None);
    assert_eq!(context.body_code, None);
}

#[test]
fn content_policy_violation_forwards_body_type_and_code() {
    let mut request = input(
        "invalid_request_error raised: content_policy_violation detected",
        Some(400),
    );
    request.body = Some(OpenaiErrorBody {
        kind: Some("custom_type".to_owned()),
        param: None,
        code: Some("custom_code".to_owned()),
        message: None,
        error: None,
    });
    let ErrorClass::ContentPolicyViolation(context) = map_openai_exception(&request) else {
        panic!("expected ContentPolicyViolation");
    };
    assert_eq!(context.body_type.as_deref(), Some("custom_type"));
    assert_eq!(context.body_code.as_deref(), Some("custom_code"));
}
