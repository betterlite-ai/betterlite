use crate::custom_code::CustomCodeGuardrailConfigModel;
use crate::custom_code::config::resolve_timeout;

#[test]
fn get_config_model_is_the_custom_code_config_model() {
    let model: CustomCodeGuardrailConfigModel = serde_json::from_str(
        r#"{"custom_code": "def apply_guardrail(inputs, request_data, input_type):\n    return allow()", "timeout": 12}"#,
    )
    .unwrap();
    assert_eq!(model.timeout.map(f64::to_bits), Some(12.0_f64.to_bits()));
    assert!(model.custom_code.contains("apply_guardrail"));
}

#[test]
fn the_config_model_defaults_the_timeout_to_thirty_seconds() {
    let model: CustomCodeGuardrailConfigModel =
        serde_json::from_str(r#"{"custom_code": "code"}"#).unwrap();
    let (seconds, budget) = resolve_timeout(model.timeout).unwrap();
    assert_eq!(seconds.to_bits(), 30.0_f64.to_bits());
    assert_eq!(budget.as_secs(), 30);
}
