#![cfg(test)]

use super::RuntimeError;
use betterlite_openai::sagemaker_completion::error::SagemakerError;

#[test]
fn runtime_errors_preserve_sagemaker_errors_and_map_http_failures() {
    let upstream = SagemakerError {
        status_code: 429,
        message: String::from("throttled"),
        headers: None,
    };
    assert_eq!(
        SagemakerError::from(RuntimeError::Sagemaker(upstream.clone())),
        upstream
    );
    let mapped = SagemakerError::from(RuntimeError::Field { field: "model_id" });
    assert_eq!(mapped.status_code, 500);
    assert!(mapped.message.contains("model_id"));
}

#[test]
fn hf_reply_error_keeps_its_wire_status() {
    let source = serde_json::from_str::<serde_json::Value>("invalid").expect_err("bad JSON");
    let mapped = SagemakerError::from(RuntimeError::HfReply(
        crate::sagemaker_embedding::hf::HfReplyError::Parse {
            status_code: 422,
            source,
        },
    ));
    assert_eq!(mapped.status_code, 422);
}
