#![cfg(test)]

use rstest::rstest;

use super::body::{AzureErrorDetail, AzureInnerError};
use super::*;

// pairs ErrorClass's own #[serde(tag = "family", rename_all = "snake_case")] label with the message each family carries, directly or inside its details struct, without touching serde_json::Value
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
            panic!("azure classification never produces this family")
        }
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
        describe(&class).0,
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

#[rstest]
#[case::status_400_has_no_label(400, "AzureException - boom")]
#[case::status_401_label(401, "AzureException AuthenticationError - boom")]
#[case::status_404_label(404, "AzureException NotFoundError - boom")]
#[case::status_408_label(408, "AzureException Timeout - boom")]
#[case::status_422_label(422, "AzureException BadRequestError - boom")]
#[case::status_429_label(429, "AzureException RateLimitError - boom")]
#[case::status_502_label(502, "AzureException BadGatewayError - boom")]
#[case::status_503_label(503, "AzureException ServiceUnavailableError - boom")]
#[case::status_504_label(504, "AzureException Timeout - boom")]
#[case::status_other_label(418, "AzureException APIError - boom")]
fn status_classification_renders_the_matching_label(#[case] status: u16, #[case] expected: &str) {
    let class = map_azure_exception(&input("boom", Some(status)));
    assert_eq!(describe(&class).1, expected);
}

#[test]
fn no_status_code_message_names_the_exception_provider() {
    let class = map_azure_exception(&input("boom", None));
    assert_eq!(describe(&class).1, "Azure APIConnectionError - boom");
}

#[test]
fn resolve_message_prefers_the_message_field_over_error_str() {
    let mut request = input("boom", Some(401));
    request.message = Some("the real message".to_owned());
    let class = map_azure_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AzureException AuthenticationError - the real message"
    );
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

#[test]
fn resolve_message_prefers_the_body_message_over_error_str() {
    let mut request = input("boom", Some(401));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: Some("from the body".to_owned()),
        error: None,
    });
    let class = map_azure_exception(&request);
    assert_eq!(
        describe(&class).1,
        "AzureException AuthenticationError - from the body"
    );
}

#[test]
fn content_policy_violation_wraps_the_message_like_litellm_does() {
    let class = map_azure_exception(&input("content_policy_violation detected", Some(400)));
    assert_eq!(
        describe(&class).1,
        "litellm.BadRequestError: litellm.ContentPolicyViolationError: content_policy_violation detected"
    );
}

#[test]
fn content_policy_forwards_its_own_type_and_code_instead_of_the_defaults() {
    let mut request = input("content_policy_violation detected", Some(400));
    request.body = Some(AzureErrorBody {
        kind: None,
        param: None,
        code: None,
        message: None,
        error: Some(AzureErrorDetail {
            kind: Some("custom_type".to_owned()),
            param: None,
            code: Some("custom_code".to_owned()),
            message: None,
            inner_error: None,
            innererror: None,
        }),
    });
    let ErrorClass::ContentPolicyViolation(context) = map_azure_exception(&request) else {
        panic!("expected ContentPolicyViolation");
    };
    assert_eq!(context.body_type.as_deref(), Some("custom_type"));
    assert_eq!(context.body_code.as_deref(), Some("custom_code"));
}
