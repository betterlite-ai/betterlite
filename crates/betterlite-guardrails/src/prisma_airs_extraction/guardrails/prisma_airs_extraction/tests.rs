#![cfg(test)]

use serde_json::json;

use crate::prisma_airs_extraction::{extract_response_text, extract_text_from_messages};

#[test]
fn extracts_last_user_or_developer_content_and_flattens_text_parts() {
    let messages = json!([
        {"role":"user", "content":"older"},
        {"role":"assistant", "content":"skip"},
        {"role":"developer", "content":[{"type":"text", "text":"one"}, {"type":"image"}, {"type":"text", "text":"two"}]}
    ]);
    assert_eq!(
        extract_text_from_messages(messages.as_array().expect("messages")),
        "one two"
    );
}

#[test]
fn extracts_all_response_content_and_tool_arguments_across_choices() {
    let response = json!({"choices":[
        {"message":{"content":"answer", "tool_calls":[{"function":{"arguments":"{\"x\":1}"}}]}},
        {"message":{"content":[{"type":"text", "text":"next"}], "function_call":{"arguments":"legacy"}}}
    ]});
    assert_eq!(
        extract_response_text(&response),
        "answer {\"x\":1} next legacy"
    );
}
