#![cfg(test)]

use super::*;

#[test]
fn request_params_parses_values_and_fallback() {
    let params = serde_json::from_value::<BTreeMap<String, Value>>(serde_json::json!({"temperature":0.5,"top_p":0.9,"max_completion_tokens":20,"stop":["x"],"seed":3})).unwrap();
    let parsed = LLMRequestParams::from_model_parameters(&params);
    assert_eq!(parsed.max_tokens, Some(20));
    assert_eq!(parsed.stop_sequences, Some(vec!["x".to_owned()]));
}
