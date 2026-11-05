#![cfg(test)]

use rstest::rstest;

use super::transform_to_a2a_response;
use crate::pydantic_ai_transformation::tests::{Source, response, uuid};

fn transform(result: &str) -> String {
    let event = transform_to_a2a_response(&response(result), "request-1", &mut Source::default());
    serde_json::to_string(&event).unwrap()
}

#[test]
fn transform_wraps_the_artifact_parts_in_a_direct_message() {
    let parts = r#"[{"kind":"text","text":"hello"},{"kind":"file","file":{"uri":"u"}}]"#;
    let output = transform(&format!(
        r#"{{"artifacts": [{{"parts": {parts}}}], "status": {{"state": "completed"}}}}"#
    ));
    assert_eq!(
        output,
        format!(
            r#"{{"id":"request-1","jsonrpc":"2.0","result":{{"kind":"message","role":"agent","parts":{parts},"messageId":"{}"}}}}"#,
            uuid(1)
        )
    );
}

#[test]
fn transform_keeps_the_history_message_id() {
    let output = transform(
        r#"{"history": [
            {"role": "user", "messageId": "question", "parts": [{"kind": "text", "text": "q"}]},
            {"role": "agent", "messageId": "answer", "parts": [{"kind": "text", "text": "a"}]}
        ]}"#,
    );
    assert_eq!(
        output,
        r#"{"id":"request-1","jsonrpc":"2.0","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":"a"}],"messageId":"answer"}}"#
    );
}

#[rstest]
#[case::empty_result("{}")]
#[case::unusable_history(r#"{"history": [{"role": "user", "parts": []}]}"#)]
#[case::empty_message(r#"{"message": {}}"#)]
fn transform_without_text_gives_one_empty_text_part(#[case] result: &str) {
    assert_eq!(
        transform(result),
        format!(
            r#"{{"id":"request-1","jsonrpc":"2.0","result":{{"kind":"message","role":"agent","parts":[{{"kind":"text","text":""}}],"messageId":"{}"}}}}"#,
            uuid(1)
        )
    );
}

#[test]
fn transform_message_field_without_parts_gives_one_empty_text_part() {
    assert_eq!(
        transform(r#"{"message": {"messageId": "m", "parts": []}}"#),
        r#"{"id":"request-1","jsonrpc":"2.0","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":""}],"messageId":"m"}}"#
    );
}
