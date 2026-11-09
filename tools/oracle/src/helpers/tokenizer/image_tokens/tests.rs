#![cfg(test)]

use serde_json::json;

use super::Input;

#[test]
fn negative_base_tokens_are_refused() {
    let call = json!({
        "calls": [{"function": "calculate_img_tokens", "data": "data:x;base64,", "base_tokens": -1_i32}]
    });
    let error = serde_json::from_value::<Input>(call).unwrap_err();
    assert!(error.to_string().contains("expected u64"));
}
