#![cfg(test)]

use rstest::rstest;

use super::body::{AzureErrorDetail, AzureInnerError};
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

fn input(error_str: &str, status_code: Option<u16>) -> AzureExceptionInput {
    AzureExceptionInput {
        model: "gpt-4".to_owned(),
        custom_llm_provider: "azure".to_owned(),
        error_str: error_str.to_owned(),
        exception_provider: "Azure".to_owned(),
        status_code,
        message: None,
        body: None,
    }
}

#[rstest]
#[case::internal_server_error_text(
    input("Internal server error occurred", None),
    "internal_server"
)]
#[case::context_window_text(
    input("This model's maximum context length is 4096", None),
    "context_window_exceeded"
)]
#[case::deployment_not_found_text(
    input("DeploymentNotFound: no such deployment", None),
    "not_found"
)]
#[case::content_policy_via_text(
    input("content_policy_violation detected in prompt", Some(400)),
    "content_policy_violation"
)]
#[case::missing_credential_text(
    input(
        "The api_key client option must be set either by passing api_key to the client or by setting",
        None,
    ),
    "authentication"
)]
#[case::connection_error_text(input("Connection error talking to Azure", None), "api_connection")]
#[case::status_401(input("boom", Some(401)), "authentication")]
#[case::status_404(input("boom", Some(404)), "not_found")]
#[case::status_408(input("boom", Some(408)), "timeout")]
#[case::status_422(input("boom", Some(422)), "bad_request")]
#[case::status_429(input("boom", Some(429)), "rate_limit")]
#[case::status_502(input("boom", Some(502)), "bad_gateway")]
#[case::status_503(input("boom", Some(503)), "service_unavailable")]
#[case::status_504(input("boom", Some(504)), "timeout")]
#[case::status_other(input("boom", Some(418)), "api")]
#[case::no_status_connection_fallback(input("boom", None), "api_connection")]
fn azure_openrouter_errors_behavior_tests_behavior(
    #[case] input: AzureExceptionInput,
    #[case] expected_family: &str,
) {
    let class = map_azure_exception(&input);
    assert_eq!(
        family_of(&class),
        expected_family,
        "input {input:?} produced {class:?}"
    );
}

#[test]
fn text_classification_outranks_status_code() {
    let class = map_azure_exception(&input("Internal server error", Some(401)));
    assert!(matches!(class, ErrorClass::InternalServer(_)));
}

#[test]
fn status_code_switch_outranks_connection_text() {
    // credential_or_connection runs before status_classification, so a status code set alongside "Connection error" still loses to the text branch
    let class = map_azure_exception(&input("Connection error", Some(404)));
    assert!(matches!(class, ErrorClass::ApiConnection(_)));
}

#[test]
fn content_policy_via_body_code_outranks_status() {
    let mut request = input("boom", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(AzureErrorDetail {
            kind: None,
            param: None,
            code: Some("content_policy_violation".to_owned()),
            message: None,
            inner_error: None,
            innererror: None,
        }),
    });
    let class = map_azure_exception(&request);
    assert!(matches!(class, ErrorClass::ContentPolicyViolation(_)));
}

#[test]
fn inner_error_responsible_ai_promotes_to_content_policy() {
    let mut request = input("boom", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(AzureErrorDetail {
            kind: None,
            param: None,
            code: Some("other".to_owned()),
            message: None,
            inner_error: Some(AzureInnerError {
                code: Some("ResponsibleAIPolicyViolation".to_owned()),
            }),
            innererror: None,
        }),
    });
    let class = map_azure_exception(&request);
    assert!(matches!(class, ErrorClass::ContentPolicyViolation(_)));
}

#[test]
fn innererror_used_only_when_inner_error_absent() {
    let mut request = input("boom", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(AzureErrorDetail {
            kind: None,
            param: None,
            code: Some("other".to_owned()),
            message: None,
            inner_error: None,
            innererror: Some(AzureInnerError {
                code: Some("ResponsibleAIPolicyViolation".to_owned()),
            }),
        }),
    });
    let class = map_azure_exception(&request);
    assert!(matches!(class, ErrorClass::ContentPolicyViolation(_)));
}

#[test]
fn inner_error_present_without_matching_code_blocks_innererror_fallback() {
    // inner_error wins over innererror whenever it is Some(_), even without a matching code
    let mut request = input("boom", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(AzureErrorDetail {
            kind: None,
            param: None,
            code: Some("other".to_owned()),
            message: None,
            inner_error: Some(AzureInnerError { code: None }),
            innererror: Some(AzureInnerError {
                code: Some("ResponsibleAIPolicyViolation".to_owned()),
            }),
        }),
    });
    let class = map_azure_exception(&request);
    assert!(!matches!(class, ErrorClass::ContentPolicyViolation(_)));
}

#[test]
fn azure_error_code_falls_back_to_top_level_code_without_error_object() {
    let mut request = input("invalid_encrypted_content body", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: Some("invalid_encrypted_content".to_owned()),
        message: None,
        error: None,
    });
    let class = map_azure_exception(&request);
    assert!(matches!(class, ErrorClass::BadRequest(_)));
}

#[test]
fn invalid_encrypted_content_body_code_routes_bad_request() {
    let mut request = input("boom", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(AzureErrorDetail {
            kind: None,
            param: None,
            code: Some("invalid_encrypted_content".to_owned()),
            message: None,
            inner_error: None,
            innererror: None,
        }),
    });
    let class = map_azure_exception(&request);
    assert!(matches!(class, ErrorClass::BadRequest(_)));
}

#[test]
fn could_not_be_verified_text_routes_bad_request() {
    let class = map_azure_exception(&input("the signature could not be verified", Some(500)));
    assert!(matches!(class, ErrorClass::BadRequest(_)));
}

#[test]
fn invalid_request_error_text_requires_no_status_or_400() {
    let matched = map_azure_exception(&input("invalid_request_error: bad field", None));
    assert!(matches!(matched, ErrorClass::BadRequest(_)));
    let also_matched = map_azure_exception(&input("invalid_request_error: bad field", Some(400)));
    assert!(matches!(also_matched, ErrorClass::BadRequest(_)));
    let unmatched = map_azure_exception(&input("invalid_request_error: bad field", Some(404)));
    assert!(matches!(unmatched, ErrorClass::NotFound(_)));
}

#[test]
fn status_400_forwards_body_type_param_code() {
    let mut request = input("boom", Some(400));
    request.body = Some(AzureErrorBody {
        kind: Some("invalid_request_error".to_owned()),
        param: Some("messages".to_owned()),
        code: Some("missing_field".to_owned()),
        message: None,
        error: None,
    });
    let ErrorClass::BadRequest(context) = map_azure_exception(&request) else {
        panic!("expected BadRequest");
    };
    assert_eq!(context.body_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(context.body_param.as_deref(), Some("messages"));
    assert_eq!(context.body_code.as_deref(), Some("missing_field"));
}
