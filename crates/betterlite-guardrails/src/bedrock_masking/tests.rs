#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn masked_outputs_stop_at_missing_text() {
    let response = json!({"outputs":[{"text":"one"},{"future":true},{"text":"three"}]});
    assert_eq!(extract_masked_texts(Some(&response)), ["one"]);
}

#[test]
fn masking_preserves_non_text_content_and_indexes_text() {
    let messages = vec![json!({"role":"user","content":[{"type":"text","text":"a"},{"type":"image"},"b"]})];
    let masked = apply_masking_to_messages(&messages, &["A".to_owned(), "B".to_owned()]);
    assert_eq!(masked[0]["content"][0]["text"], "A");
    assert_eq!(masked[0]["content"][1]["type"], "image");
    assert_eq!(masked[0]["content"][2], "B");
}

#[test]
fn response_choice_masking_covers_message_delta_and_text() {
    let mut response = json!({"choices":[{"message":{"content":"a"}},{"delta":{"content":"b"}},{"text":"c"}]});
    apply_masking_to_response(&mut response, &["A".to_owned(), "B".to_owned(), "C".to_owned()]);
    assert_eq!(response["choices"][0]["message"]["content"], "A");
    assert_eq!(response["choices"][1]["delta"]["content"], "B");
    assert_eq!(response["choices"][2]["text"], "C");
}
