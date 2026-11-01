#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(
    custom_llm_provider: &str,
    error_str: &str,
    status_code: Option<u16>,
) -> SagemakerExceptionInput {
    SagemakerExceptionInput {
        model: "huggingface-llama".to_owned(),
        custom_llm_provider: custom_llm_provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
    }
}

// RateLimit and Timeout nest the shared fields under "context"; every other family keeps them at the top level
macro_rules! provider_of {
    ($value:expr) => {
        $value["provider"]
            .as_str()
            .or_else(|| $value["context"]["provider"].as_str())
    };
}

#[rstest]
#[case::branch_1046_missing_credentials(
    "sagemaker",
    "Unable to locate credentials for this request",
    None,
    Some("bad_request"),
    Some("sagemaker")
)]
#[case::branch_1053_best_of_validation(
    "sagemaker",
    "Input validation error: `best_of` must be > 0 and <= 2",
    None,
    Some("bad_request"),
    Some("sagemaker")
)]
#[case::branch_1060_inputs_plus_max_new_tokens(
    "sagemaker",
    "`inputs` tokens + `max_new_tokens` must be <= 2048",
    None,
    Some("context_window_exceeded"),
    Some("sagemaker")
)]
#[case::branch_1060_instance_type_capacity(
    "sagemaker_chat",
    "use an instance type with more CPU capacity or memory",
    None,
    Some("context_window_exceeded"),
    Some("sagemaker")
)]
// hasattr(original_exception, "status_code") is false: nothing in the elif chain matches and the function returns None
#[case::branch_1070_hasattr_false_falls_through(
    "sagemaker",
    "an unmatched failure",
    None,
    None,
    None
)]
#[case::branch_1071_status_500(
    "sagemaker",
    "an unmatched failure",
    Some(500),
    Some("service_unavailable"),
    Some("sagemaker")
)]
#[case::branch_1081_status_401(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(401),
    Some("authentication"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1088_status_400(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(400),
    Some("bad_request"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1095_status_404(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(404),
    Some("not_found"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1102_status_408(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(408),
    Some("timeout"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1109_status_422(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(422),
    Some("bad_request"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1109_status_424(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(424),
    Some("bad_request"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1117_status_429(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(429),
    Some("rate_limit"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1125_status_503(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(503),
    Some("service_unavailable"),
    Some("my_sagemaker_proxy")
)]
#[case::branch_1133_status_504(
    "my_sagemaker_proxy",
    "an unmatched failure",
    Some(504),
    Some("timeout"),
    Some("my_sagemaker_proxy")
)]
#[case::generic_fallback_unmatched_status(
    "sagemaker",
    "an unmatched failure",
    Some(999),
    None,
    None
)]
fn aws_errors_behavior_tests_behavior(
    #[case] custom_llm_provider: &str,
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_family: Option<&str>,
    #[case] expected_provider: Option<&str>,
) {
    let mapped = map_sagemaker_exception(&input(custom_llm_provider, error_str, status_code));
    match expected_family {
        None => assert!(mapped.is_none()),
        Some(family) => {
            let class = mapped.expect("a matching predicate must produce a class");
            let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
            assert_eq!(value["family"], serde_json::json!(family));
            let expected =
                expected_provider.expect("a matched case always carries an expected provider");
            assert_eq!(provider_of!(value), Some(expected));
        }
    }
}

#[test]
fn message_carries_the_family_specific_reason() {
    let mapped = map_sagemaker_exception(&input(
        "sagemaker",
        "Unable to locate credentials for this request",
        None,
    ))
    .expect("the missing-credentials predicate matches");
    let value = serde_json::to_value(&mapped).expect("ErrorClass always serializes");
    assert_eq!(
        value["message"],
        serde_json::json!("sagemaker: the provider rejected the request body")
    );
}

#[test]
fn text_predicate_wins_over_a_competing_status_code() {
    let mapped = map_sagemaker_exception(&input(
        "my_sagemaker_proxy",
        "Unable to locate credentials for this request",
        Some(500),
    ))
    .expect("the text predicate matches before status_code is ever consulted");
    let value = serde_json::to_value(&mapped).expect("ErrorClass always serializes");
    assert_eq!(value["family"], serde_json::json!("bad_request"));
    assert_eq!(provider_of!(value), Some("sagemaker"));
}
