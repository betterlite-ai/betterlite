#![cfg(test)]

use super::build_a2a_message_response;
use crate::watsonx_transform::tests::{Source, uuid};

#[test]
fn message_response_is_an_agent_message_with_a_fresh_message_id() {
    let mut source = Source::default();
    let response = build_a2a_message_response("req-1", "hello world", &mut source);
    let expected = format!(
        r#"{{"id":"req-1","jsonrpc":"2.0","result":{{"kind":"message","role":"agent","parts":[{{"kind":"text","text":"hello world"}}],"messageId":"{}"}}}}"#,
        uuid(1)
    );
    assert_eq!(serde_json::to_string(&response).unwrap(), expected);
}

#[test]
fn every_message_response_takes_a_new_message_id() {
    let mut source = Source::default();
    let first = build_a2a_message_response("req-1", "same", &mut source);
    let second = build_a2a_message_response("req-1", "same", &mut source);
    assert_eq!(first.result.message_id, uuid(1));
    assert_eq!(second.result.message_id, uuid(2));
}

#[test]
fn empty_text_is_still_a_text_part() {
    let response = build_a2a_message_response("req-2", "", &mut Source::default());
    assert_eq!(response.result.parts.len(), 1);
    assert_eq!(response.result.parts[0].text.as_deref(), Some(""));
}
