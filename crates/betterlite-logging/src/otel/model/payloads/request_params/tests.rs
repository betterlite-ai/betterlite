#![cfg(test)]

use super::*;

#[test]
fn request_params_preserve_small_float_values() {
    let fields = serde_json::from_str(r#"{"temperature":1e-40}"#).unwrap();
    let parsed = LLMRequestParams::from_model_parameters(&fields);
    assert_eq!(
        parsed.temperature.map(f64::to_bits),
        Some(1e-40_f64.to_bits())
    );
}

#[test]
fn request_params_parses_values_and_fallback() {
    let params = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"temperature":0.5,"top_p":0.9,"max_completion_tokens":20,"stop":["x"],"seed":3}"#,
    )
    .unwrap();
    let parsed = LLMRequestParams::from_model_parameters(&params);
    assert_eq!(parsed.max_tokens, Some(20));
    assert_eq!(parsed.stop_sequences, Some(vec!["x".to_owned()]));
}

#[rstest::rstest]
#[case::absent("{}", None)]
#[case::null(r#"{"max_tokens":null,"max_completion_tokens":12}"#, Some(12))]
#[case::invalid(r#"{"max_tokens":{},"max_completion_tokens":12}"#, Some(12))]
#[case::zero(r#"{"max_tokens":0,"max_completion_tokens":12}"#, Some(0))]
#[case::primary(r#"{"max_tokens":9,"max_completion_tokens":12}"#, Some(9))]
fn max_tokens_fallback(#[case] raw: &str, #[case] expected: Option<i64>) {
    let parsed = LLMRequestParams::from_model_parameters(&serde_json::from_str(raw).unwrap());
    assert_eq!(parsed.max_tokens, expected);
    assert!(parsed.temperature.is_none());
    assert!(parsed.stop_sequences.is_none());
}

#[test]
fn request_params_wraps_string_stop() {
    let params = serde_json::from_str(r#"{"stop":"END"}"#).unwrap();
    let parsed = LLMRequestParams::from_model_parameters(&params);
    assert_eq!(parsed.stop_sequences, Some(vec!["END".to_owned()]));
}
