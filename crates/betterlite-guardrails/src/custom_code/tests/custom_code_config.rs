use std::time::Duration;

use rstest::rstest;

use crate::custom_code::CustomCodeGuardrailConfigModel;
use crate::custom_code::config::resolve_timeout;
use crate::custom_code::error::CustomCodeGuardrailError;

#[test]
fn the_config_model_reads_custom_code_and_an_optional_timeout() {
    let model: CustomCodeGuardrailConfigModel = serde_json::from_str(
        r#"{"custom_code": "def apply_guardrail(inputs, request_data, input_type): pass"}"#,
    )
    .unwrap();
    assert_eq!(
        model.custom_code,
        "def apply_guardrail(inputs, request_data, input_type): pass"
    );
    assert_eq!(model.timeout, None);

    let explicit: CustomCodeGuardrailConfigModel =
        serde_json::from_str(r#"{"custom_code": "code", "timeout": 12.5, "unknown": true}"#)
            .unwrap();
    assert_eq!(explicit.timeout.map(f64::to_bits), Some(12.5_f64.to_bits()));

    let null: CustomCodeGuardrailConfigModel =
        serde_json::from_str(r#"{"custom_code": "code", "timeout": null}"#).unwrap();
    assert_eq!(null.timeout, None);
}

#[test]
fn the_config_model_requires_custom_code() {
    let error =
        serde_json::from_str::<CustomCodeGuardrailConfigModel>(r#"{"timeout": 5}"#).unwrap_err();
    assert!(error.to_string().contains("custom_code"));
}

#[rstest]
#[case::absent(None, 30.0_f64)]
#[case::explicit_seconds(Some(0.5_f64), 0.5_f64)]
#[case::whole_seconds(Some(45.0_f64), 45.0_f64)]
fn a_missing_timeout_resolves_to_thirty_seconds(
    #[case] timeout: Option<f64>,
    #[case] expected: f64,
) {
    let (seconds, budget) = resolve_timeout(timeout).unwrap();
    assert_eq!(seconds.to_bits(), expected.to_bits());
    assert_eq!(budget, Duration::try_from_secs_f64(expected).unwrap());
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::negative(-1.5_f64)]
#[case::nan(f64::NAN)]
#[case::infinite(f64::INFINITY)]
fn a_timeout_that_is_not_a_positive_number_is_refused(#[case] timeout: f64) {
    let error = resolve_timeout(Some(timeout)).unwrap_err();
    assert!(matches!(
        error,
        CustomCodeGuardrailError::NonPositiveTimeout { .. }
    ));
}
