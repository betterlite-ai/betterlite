#![cfg(test)]

use serde_json::json;

use super::answer;
use super::scripted::Input;

fn scripted_input() -> Input {
    serde_json::from_value(json!({
        "vector_store_id": "vs_a",
        "custom_llm_provider": "openai",
        "kwargs": {"api_key": "sk-caller-secret", "timeout": 7},
        "registry": {
            "result": {"api_key": "sk-registry-secret"},
            "error": null
        }
    }))
    .unwrap()
}

#[test]
fn input_debug_hides_credential_values() {
    let rendered = format!("{:?}", scripted_input());
    assert!(!rendered.contains("sk-caller-secret"));
    assert!(!rendered.contains("sk-registry-secret"));
}

#[test]
fn answer_debug_hides_credential_values() {
    let rendered = format!("{:?}", answer(&scripted_input()));
    assert!(!rendered.contains("sk-caller-secret"));
    assert!(!rendered.contains("sk-registry-secret"));
}
