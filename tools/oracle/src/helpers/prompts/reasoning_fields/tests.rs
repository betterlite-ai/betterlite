#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn extracts_the_base64_payload() {
    let input: Input = serde_json::from_str(
        r#"{"operation":"extract_base64_data","image_url":"data:image/png;base64,abc"}"#,
    )
    .unwrap();
    let value = Op::run(&input).unwrap();
    assert_eq!(value, serde_json::json!("abc"));
}

#[test]
fn splits_a_tagged_message_into_reasoning_and_content() {
    let input: Input = serde_json::from_str(
        r#"{"operation":"parse_content_for_reasoning","message_text":"<think>because</think>visible"}"#,
    )
    .unwrap();
    let value = Op::run(&input).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"reasoning_content": "because", "content": "visible"})
    );
}
