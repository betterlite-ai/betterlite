use betterlite_prompts::arguments::json::Json;

use super::{list_fixture, object_fixture};
use crate::message_replay::{BlockStrip, strip_blocks, strip_thinking_from_request};

fn object(text: &str) -> Json {
    Json::Object(object_fixture(text))
}

#[test]
fn thinking_strip_drops_an_initially_empty_content_list() {
    let messages = list_fixture(r#"[{"role":"assistant","content":[]}]"#);
    assert_eq!(
        strip_blocks(&messages, BlockStrip::Thinking),
        list_fixture("[]")
    );
}

#[test]
fn encrypted_strip_retains_an_initially_empty_content_list() {
    let messages = list_fixture(r#"[{"role":"assistant","content":[]}]"#);
    assert_eq!(
        strip_blocks(&messages, BlockStrip::EncryptedReasoning),
        messages
    );
}

#[test]
fn empty_content_strip_retains_an_initially_empty_content_list() {
    let messages = list_fixture(r#"[{"role":"assistant","content":[]}]"#);
    assert_eq!(strip_blocks(&messages, BlockStrip::EmptyContent), messages);
}

#[test]
fn strips_keep_non_object_messages_and_non_list_content() {
    let messages = list_fixture(
        r#"["text",{"role":"user","content":"hi"},{"role":"user"},{"role":"user","content":null},{"role":"user","content":{}}]"#,
    );
    assert_eq!(strip_blocks(&messages, BlockStrip::Thinking), messages);
    assert_eq!(
        strip_blocks(&messages, BlockStrip::EncryptedReasoning),
        messages
    );
    assert_eq!(strip_blocks(&messages, BlockStrip::EmptyContent), messages);
}

#[test]
fn thinking_strip_drops_a_message_emptied_by_removal() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"a"},{"type":"redacted_thinking","data":"b"}]},{"role":"user","content":"hi"}]"#,
    );
    assert_eq!(
        strip_blocks(&messages, BlockStrip::Thinking),
        list_fixture(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn encrypted_strip_drops_a_message_whose_blocks_were_all_removed() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"a","signature":"litellm_encrypted_reasoning:x"}]},{"role":"user","content":"hi"}]"#,
    );
    assert_eq!(
        strip_blocks(&messages, BlockStrip::EncryptedReasoning),
        list_fixture(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn empty_content_strip_drops_a_message_emptied_by_removal() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"  "}]},{"role":"user","content":"hi"}]"#,
    );
    assert_eq!(
        strip_blocks(&messages, BlockStrip::EmptyContent),
        list_fixture(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[test]
fn thinking_strip_removes_both_thinking_kinds_and_keeps_others() {
    let messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"a"},{"type":"redacted_thinking","data":"b"},{"type":"text","text":"keep"},"opaque"]}]"#,
    );
    assert_eq!(
        strip_blocks(&messages, BlockStrip::Thinking),
        list_fixture(
            r#"[{"role":"assistant","content":[{"type":"text","text":"keep"},"opaque"]}]"#
        )
    );
}

#[test]
fn empty_content_strip_keeps_redacted_thinking() {
    let messages = list_fixture(
        r#"[{"role":"user","content":[{"type":"redacted_thinking","data":"b"},{"type":"text","text":"keep"}]}]"#,
    );
    assert_eq!(strip_blocks(&messages, BlockStrip::EmptyContent), messages);
}

#[test]
fn strip_result_preserves_the_message_key_order() {
    let messages = list_fixture(
        r#"[{"role":"assistant","meta":1,"content":[{"type":"text","text":""},{"type":"text","text":"x"}],"tail":2}]"#,
    );
    let stripped = strip_blocks(&messages, BlockStrip::EmptyContent);
    let fields = stripped.first().and_then(Json::as_object).unwrap();
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(keys, ["role", "meta", "content", "tail"]);
}

#[test]
fn thinking_request_repair_removes_the_key_alone_when_messages_is_missing() {
    let mut data = object_fixture(r#"{"thinking":{"type":"enabled"},"model":"m"}"#);
    strip_thinking_from_request(&mut data);
    assert_eq!(Json::Object(data), object(r#"{"model":"m"}"#));
}

#[test]
fn thinking_request_repair_keeps_a_non_list_messages_value() {
    let mut data = object_fixture(r#"{"messages":"raw","thinking":1}"#);
    strip_thinking_from_request(&mut data);
    assert_eq!(Json::Object(data), object(r#"{"messages":"raw"}"#));
}

#[test]
fn thinking_request_repair_strips_the_message_list() {
    let mut data = object_fixture(
        r#"{"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"a"},{"type":"text","text":"b"}]}],"thinking":{"type":"enabled"}}"#,
    );
    strip_thinking_from_request(&mut data);
    assert_eq!(
        Json::Object(data),
        object(r#"{"messages":[{"role":"assistant","content":[{"type":"text","text":"b"}]}]}"#)
    );
}

#[test]
fn thinking_request_repair_removes_a_null_key() {
    let mut data = object_fixture(r#"{"thinking":null,"messages":[]}"#);
    strip_thinking_from_request(&mut data);
    assert_eq!(Json::Object(data), object(r#"{"messages":[]}"#));
}

#[test]
fn thinking_request_repair_preserves_the_remaining_key_order() {
    let mut data = object_fixture(r#"{"model":"m","messages":[],"thinking":{},"temperature":1}"#);
    strip_thinking_from_request(&mut data);
    let keys: Vec<&str> = data.keys().map(String::as_str).collect();
    assert_eq!(keys, ["model", "messages", "temperature"]);
}
