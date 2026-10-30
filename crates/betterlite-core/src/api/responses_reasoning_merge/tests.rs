#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn reasoning_before_answer() {
    let input = vec![
        json!({"role":"assistant","content":null,"reasoning_content":"a","thinking_blocks":["A"]}),
        json!({"role":"assistant","content":null,"reasoning_content":"b","thinking_blocks":["B"]}),
        json!({"role":"assistant","content":"answer","reasoning_content":"c","thinking_blocks":["C"],"tool_calls":[1],"extra":2}),
    ];
    let output = merge_reasoning(&input);
    assert_eq!(
        output,
        vec![
            json!({"role":"assistant","content":"answer","reasoning_content":"a\nb\nc","thinking_blocks":["A","B","C"],"tool_calls":[1],"extra":2})
        ]
    );
}

#[test]
fn reasoning_boundaries() {
    let output = merge_reasoning(&[
        json!({"role":"assistant","content":null,"reasoning_content":"a","unrelated":true}),
        json!({"role":"user","content":"q"}),
        json!({"role":"assistant","content":null,"thinking_blocks":["B"]}),
    ]);
    assert_eq!(
        output,
        vec![
            json!({"role":"assistant","content":null,"reasoning_content":"a"}),
            json!({"role":"user","content":"q"}),
            json!({"role":"assistant","content":null,"thinking_blocks":["B"]})
        ]
    );
}

#[test]
fn trailing_assistant() {
    let output = merge_trailing(
        &[json!({"role":"assistant","content":"a","tool_calls":[{"id":"c"}],"keep":true})],
        &[json!({"role":"assistant","content":"b","new":true})],
        false,
    )
    .unwrap();
    assert_eq!(
        output,
        Some(
            json!({"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"tool_calls":[{"id":"c"}],"keep":true})
        )
    );
}

#[test]
fn trailing_rejections() {
    assert_eq!(merge_trailing(&[], &[], false).unwrap(), None);
    assert_eq!(
        merge_trailing(
            &[json!({"role":"assistant","content":"a"})],
            &[json!({"role":"assistant","content":"b"})],
            false
        )
        .unwrap(),
        None
    );
    assert!(matches!(
        merge_trailing(
            &[json!({"role":"assistant","content":[1],"tool_calls":[1]})],
            &[json!({"role":"assistant","content":"b"})],
            false
        ),
        Err(MergeError::InvalidContent)
    ));
}
