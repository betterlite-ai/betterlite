use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::block_code_execution::error_handling::{
    EXECUTION_REQUEST_LANGUAGE, block_failure, block_message,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use rstest::rstest;

use crate::block_code_execution::support::{guardrail, params, request_data};

#[test]
fn an_execution_request_message_names_the_request() {
    assert_eq!(
        block_message(EXECUTION_REQUEST_LANGUAGE),
        "Content blocked: execution request detected"
    );
}

#[rstest]
#[case::python("python")]
#[case::javascript("javascript")]
#[case::unknown("unknown")]
fn a_code_block_message_names_the_language(#[case] language: &str) {
    assert_eq!(
        block_message(language),
        format!("Content blocked: executable code block detected (language: {language})")
    );
}

#[test]
fn a_response_block_is_a_400_with_the_detail_object() {
    let mut configured = params();
    configured.guardrail_name = Some("mine".to_owned());
    let built = guardrail(configured);
    let failure = block_failure(
        &built,
        "python",
        GuardrailInputType::Response,
        &request_data(),
    );
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a response block must be an object failure");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Content blocked: executable code block detected (language: python)".to_owned()
        ))
    );
    assert_eq!(
        detail.get("guardrail"),
        Some(&GuardrailValue::Text("mine".to_owned()))
    );
    assert_eq!(
        detail.get("language"),
        Some(&GuardrailValue::Text("python".to_owned()))
    );
}

#[test]
fn a_request_block_passes_through_with_the_violation() {
    let built = guardrail(params());
    let failure = block_failure(
        &built,
        EXECUTION_REQUEST_LANGUAGE,
        GuardrailInputType::Request,
        &request_data(),
    );
    let GuardrailFailure::ModifyResponse(detail) = failure else {
        panic!("a request block must be a passthrough failure");
    };
    assert_eq!(
        detail.message,
        "Content blocked: execution request detected"
    );
    assert_eq!(detail.model, "gpt-4o");
    assert_eq!(
        detail.guardrail_name.as_deref(),
        Some("block_code_execution")
    );
    assert_eq!(
        detail.detection_info.get("language"),
        Some(&GuardrailValue::Text(EXECUTION_REQUEST_LANGUAGE.to_owned()))
    );
}
