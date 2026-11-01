#![cfg(test)]

use rstest::rstest;

use super::dispatch_native;
use crate::dispatch::RawException;
use crate::error::ErrorClass;
use crate::openai::body::{OpenaiErrorBody, OpenaiErrorDetail};

fn raw(provider: &str, error_str: &str, status_code: Option<u16>) -> Box<RawException> {
    Box::new(RawException {
        model: "a-model".to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::new(),
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
#[case::anthropic_elif_arm(raw("anthropic", "Invalid API Key", None), Some("authentication"))]
#[case::anthropic_text_alias(
    raw("anthropic_text", "Invalid API Key", None),
    Some("authentication")
)]
#[case::replicate_elif_arm(
    raw("replicate", "Incorrect authentication token", None),
    Some("authentication")
)]
#[case::openai_like_predibase(
    raw("predibase", "authorization denied for", None),
    Some("authentication")
)]
#[case::openai_like_databricks(raw("databricks", "token_quota_reached", None), Some("rate_limit"))]
#[case::openai_like_lemonade(raw("lemonade", "token_quota_reached", None), Some("rate_limit"))]
#[case::openai_like_watsonx(raw("watsonx", "token_quota_reached", None), Some("rate_limit"))]
#[case::bedrock_elif_arm(
    raw("bedrock", "AccessDeniedException", None),
    Some("permission_denied")
)]
#[case::bedrock_mantle_alias(
    raw("bedrock_mantle", "AccessDeniedException", None),
    Some("permission_denied")
)]
#[case::sagemaker_elif_arm(raw("sagemaker", "Model not ready", Some(404)), Some("not_found"))]
#[case::sagemaker_chat_alias(
    raw("sagemaker_chat", "Model not ready", Some(404)),
    Some("not_found")
)]
#[case::vertex_ai_elif_arm(
    raw("vertex_ai", "Vertex AI API has not been used in project", None),
    Some("bad_request")
)]
#[case::vertex_ai_beta_alias(
    raw("vertex_ai_beta", "Vertex AI API has not been used in project", None),
    Some("bad_request")
)]
#[case::gemini_alias(
    raw("gemini", "Vertex AI API has not been used in project", None),
    Some("bad_request")
)]
#[case::cloudflare_elif_arm(
    raw("cloudflare", "Authentication error", None),
    Some("authentication")
)]
#[case::cohere_elif_arm(
    raw("cohere", "too many tokens", None),
    Some("context_window_exceeded")
)]
#[case::cohere_chat_alias(
    raw("cohere_chat", "too many tokens", None),
    Some("context_window_exceeded")
)]
#[case::huggingface_elif_arm(raw("huggingface", "Rate limit reached", None), Some("rate_limit"))]
#[case::nlp_cloud_elif_arm(
    raw("nlp_cloud", "detail: value is not a valid", None),
    Some("bad_request")
)]
#[case::aleph_alpha_elif_arm(
    raw(
        "aleph_alpha",
        "This is longer than the model's maximum context length",
        None
    ),
    Some("context_window_exceeded")
)]
#[case::ollama_elif_arm(
    raw("ollama", "Failed to establish a new connection", None),
    Some("service_unavailable")
)]
#[case::ollama_chat_alias(
    raw("ollama_chat", "Failed to establish a new connection", None),
    Some("service_unavailable")
)]
#[case::vllm_elif_arm(raw("vllm", "connection refused", Some(0)), Some("api_connection"))]
#[case::azure_elif_arm(raw("azure", "Internal server error", None), Some("internal_server"))]
#[case::azure_text_alias(
    raw("azure_text", "Internal server error", None),
    Some("internal_server")
)]
#[case::openrouter_arm(raw("openrouter", "boom", Some(404)), Some("not_found"))]
#[case::unrecognized_provider_is_not_native(raw("some-unknown-provider", "boom", None), None)]
fn native_errors_native_error_tests_behavior(
    #[case] input: Box<RawException>,
    #[case] expected_family: Option<&str>,
) {
    let class = dispatch_native(&input, &input.error_str);
    assert_eq!(
        class.as_ref().map(family),
        expected_family,
        "produced {class:?}"
    );
}

#[test]
fn ai21_elif_arm_still_classifies_when_called_directly_even_though_dispatch_never_reaches_it() {
    let mut input = raw("ai21", "boom", Some(401));
    input.message = Some("boom".to_owned());
    let class = dispatch_native(&input, &input.error_str);
    assert_eq!(
        family(&class.expect("ai21 classifies status 401")),
        "authentication"
    );
}

#[test]
fn together_ai_elif_arm_still_classifies_when_called_directly_even_though_dispatch_never_reaches_it()
 {
    let input = raw("together_ai", "A timeout occurred", Some(408));
    let class = dispatch_native(&input, &input.error_str);
    assert_eq!(
        family(&class.expect("together_ai classifies a timeout")),
        "timeout"
    );
}

#[test]
fn huggingface_classifier_returning_none_falls_through_dispatch_native() {
    let input = raw("huggingface", "an unrelated message", None);
    assert_eq!(dispatch_native(&input, &input.error_str), None);
}

#[test]
fn ollama_original_exception_is_always_the_text_variant_never_the_dict_variant() {
    let input = raw("ollama", "Failed to establish a new connection", None);
    let class = dispatch_native(&input, &input.error_str).expect("ollama classifies this text");
    let ErrorClass::ServiceUnavailable(context) = class else {
        panic!("expected ServiceUnavailable");
    };
    assert!(
        context
            .message
            .contains("Failed to establish a new connection")
    );
}

#[test]
fn azure_body_is_converted_from_the_openai_shaped_raw_exception_body() {
    let mut input = raw("azure", "content_policy_violation", None);
    input.body = Some(OpenaiErrorBody {
        kind: Some("invalid_request_error".to_owned()),
        param: None,
        code: Some("content_policy_violation".to_owned()),
        message: None,
        error: Some(OpenaiErrorDetail {
            message: Some("blocked by policy".to_owned()),
        }),
    });
    let class = dispatch_native(&input, &input.error_str).expect("azure always classifies");
    let ErrorClass::ContentPolicyViolation(context) = class else {
        panic!(
            "expected ContentPolicyViolation, azure's body-derived error code must survive the conversion"
        )
    };
    assert_eq!(
        context.body_code.as_deref(),
        Some("content_policy_violation")
    );
}

#[test]
fn azure_body_conversion_is_none_when_the_raw_exception_carries_no_body() {
    let input = raw("azure", "boom", Some(500));
    let class = dispatch_native(&input, &input.error_str).expect("azure always classifies");
    let ErrorClass::Api(details) = class else {
        panic!("expected Api(500) from the status table, with no body to convert")
    };
    assert_eq!(details.context.body_code, None);
}

#[test]
fn azure_body_top_level_code_alone_drives_content_policy_classification() {
    let mut input = raw("azure", "boom underlying failure", None);
    input.body = Some(OpenaiErrorBody {
        kind: Some("custom_kind_xyz".to_owned()),
        param: None,
        code: Some("content_policy_violation".to_owned()),
        message: None,
        error: None,
    });
    let class = dispatch_native(&input, &input.error_str).expect("azure always classifies");
    let ErrorClass::ContentPolicyViolation(context) = class else {
        panic!("expected ContentPolicyViolation, driven by the body's top-level code alone")
    };
    assert_eq!(context.body_type.as_deref(), Some("custom_kind_xyz"));
}
