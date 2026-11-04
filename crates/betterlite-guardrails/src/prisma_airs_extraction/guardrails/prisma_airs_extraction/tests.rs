#![cfg(test)]

use crate::prisma_airs_extraction::{
    Message, Response, extract_response_text, extract_text_from_messages,
};

#[test]
fn extracts_last_user_or_developer_content_and_flattens_text_parts() {
    let messages: Vec<Message> = serde_json::from_str(
        r#"[
            {"role":"user", "content":"older"},
            {"role":"assistant", "content":"skip"},
            {"role":"developer", "content":[{"type":"text", "text":"one"}, {"type":"image"}, {"type":"text", "text":"two"}]}
        ]"#,
    )
    .expect("messages");
    assert_eq!(extract_text_from_messages(&messages), "one two");
}

#[test]
fn extracts_all_response_content_and_tool_arguments_across_choices() {
    let response: Response = serde_json::from_str(
        r#"{"choices":[
            {"message":{"content":"answer", "tool_calls":[{"function":{"arguments":"{\"x\":1}"}}]}},
            {"message":{"content":[{"type":"text", "text":"next"}], "function_call":{"arguments":"legacy"}}}
        ]}"#,
    )
    .expect("response");
    assert_eq!(
        extract_response_text(&response),
        "answer {\"x\":1} next legacy"
    );
}

#[rstest::rstest]
#[case::empty_string(r#""""#, "older")]
#[case::empty_parts("[]", "older")]
#[case::image_only(r#"[{"type":"image_url","image_url":{"url":"image"}}]"#, "")]
#[case::absent("null", "older")]
fn falsy_latest_content_falls_back_to_an_older_turn(#[case] content: &str, #[case] expected: &str) {
    let input = format!(
        r#"[{{"role":"user","content":"older"}},{{"role":"developer","content":{content}}}]"#
    );
    let messages: Vec<Message> = serde_json::from_str(&input).expect("messages");
    assert_eq!(extract_text_from_messages(&messages), expected);
}
