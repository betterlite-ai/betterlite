use indexmap::IndexMap;
use rstest::rstest;

use super::super::{BatchProviderCancelInput, CancelCall, CancelProvider};

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[case::azure("azure")]
#[case::vertex_ai("vertex_ai")]
#[case::bedrock("bedrock")]
#[case::xai("xai")]
#[case::other("mistral")]
fn cancel_provider_reads_its_wire_name(#[case] name: &str) {
    let text = format!(r#"{{"cancel": {{"batch_id": "batch_1", "provider": "{name}"}}}}"#);
    let decoded: BatchProviderCancelInput = serde_json::from_str(&text).expect("decode");
    let BatchProviderCancelInput::Cancel(call) = decoded else {
        panic!("the fixture names a cancel call");
    };
    assert_eq!(call.provider, CancelProvider(name.to_owned()));
}

#[test]
fn batch_provider_cancel_input_decodes_the_case_json() {
    let decoded: BatchProviderCancelInput = serde_json::from_str(
        r#"{"cancel": {
            "batch_id": "batch_1",
            "model": null,
            "provider": "vertex_ai",
            "extra_headers": {"x-one": "1"},
            "extra_body": null,
            "params": {},
            "call": {"async": {"model_in_kwargs": "gpt-4o-mini"}}
        }}"#,
    )
    .expect("decode");
    let BatchProviderCancelInput::Cancel(call) = decoded else {
        panic!("the fixture names a cancel call");
    };
    assert_eq!(call.provider, CancelProvider("vertex_ai".to_owned()));
    assert_eq!(
        call.extra_headers,
        Some(IndexMap::from([("x-one".to_owned(), "1".to_owned())]))
    );
    assert_eq!(
        call.call,
        CancelCall::Async {
            model_in_kwargs: Some("gpt-4o-mini".to_owned()),
        }
    );
}

#[test]
fn async_invoke_rejects_a_status_without_the_arns() {
    let decoded: Result<BatchProviderCancelInput, _> = serde_json::from_str(
        r#"{"async_invoke": {
            "modelArn": "arn:aws:bedrock:us-east-1::foundation-model/marengo",
            "status": "Completed"
        }}"#,
    );
    let missing_invocation = decoded.expect_err("the invocation ARN is required");
    assert!(
        missing_invocation.to_string().contains("invocationArn"),
        "{missing_invocation}"
    );
    let missing_model: Result<BatchProviderCancelInput, _> = serde_json::from_str(
        r#"{"async_invoke": {
            "invocationArn": "arn:aws:bedrock:us-east-1:1:async-invoke/abc",
            "status": "Completed"
        }}"#,
    );
    let error = missing_model.expect_err("the model ARN is required");
    assert!(error.to_string().contains("modelArn"), "{error}");
}

#[test]
fn batch_provider_cancel_refuses_a_service_account_object() {
    let decoded: Result<BatchProviderCancelInput, _> = serde_json::from_str(
        r#"{"cancel": {
            "batch_id": "batch_1",
            "provider": "vertex_ai",
            "params": {"vertex_credentials": {"type": "service_account"}}
        }}"#,
    );
    assert!(decoded.is_err(), "the credential arrives as text");
}
