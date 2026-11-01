#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(status_code: Option<u16>) -> VllmExceptionInput {
    VllmExceptionInput {
        model: "vllm/some-model".to_owned(),
        status_code,
        message: Some("boom".to_owned()),
    }
}

#[rstest]
#[case::status_zero(input(Some(0)), true)]
#[case::status_nonzero_falls_through(input(Some(418)), false)]
#[case::no_status_code_falls_through(input(None), false)]
fn native_errors_b_behavior_tests_behavior(
    #[case] input: VllmExceptionInput,
    #[case] expect_connection_error: bool,
) {
    let mapped = map_vllm_exception(&input);
    if expect_connection_error {
        assert!(matches!(
            mapped.expect("status 0 always raises"),
            ErrorClass::ApiConnection(_)
        ));
    } else {
        assert!(
            mapped.is_none(),
            "expected no classification, got {mapped:?}"
        );
    }
}

#[test]
fn missing_message_never_panics() {
    // the branch reads original_exception.message unconditionally and crashes without it in LiteLLM; betterlite falls back to an empty description instead (docs/deviations/B242.yaml)
    let class = map_vllm_exception(&VllmExceptionInput {
        model: "vllm/some-model".to_owned(),
        status_code: Some(0),
        message: None,
    });
    let ErrorClass::ApiConnection(details) = class.expect("status 0 always raises") else {
        panic!("expected ApiConnection");
    };
    assert_eq!(details.message, "vllm: ");
}
