#![cfg(test)]

use rstest::rstest;

use super::{ResponseText, extract_response_text};
use crate::pydantic_ai_transformation::tests::{Source, response, text_part, uuid};
use crate::pydantic_ai_transformation::wire::TaskResponse;

fn extract(result: &str) -> ResponseText {
    extract_response_text(&response(result), &mut Source::default())
}

#[test]
fn extract_prefers_artifacts_over_history_and_message() {
    let extracted = extract(
        r#"{
            "artifacts": [{"parts": [{"kind": "text", "text": "from artifact"}]}],
            "history": [{"role": "agent", "messageId": "m1", "parts": [{"kind": "text", "text": "from history"}]}],
            "message": {"messageId": "m2", "parts": [{"kind": "text", "text": "from message"}]}
        }"#,
    );
    assert_eq!(
        extracted,
        ResponseText {
            text: "from artifact".to_owned(),
            message_id: uuid(1),
            parts: vec![text_part("from artifact")],
        }
    );
}

#[test]
fn extract_returns_every_part_of_the_matching_artifact() {
    let extracted = extract(
        r#"{"artifacts": [
            {"parts": [{"kind": "text", "text": ""}, {"kind": "file", "file": {"uri": "u"}}]},
            {"parts": [{"kind": "data", "data": {}}, {"kind": "text", "text": "first"}, {"kind": "text", "text": "second"}]}
        ]}"#,
    );
    assert_eq!(extracted.text, "first");
    let kinds = extracted
        .parts
        .iter()
        .map(|part| part.kind.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(kinds, [Some("data"), Some("text"), Some("text")]);
}

#[rstest]
#[case::no_artifacts(r#""artifacts": []"#)]
#[case::empty_text(r#""artifacts": [{"parts": [{"kind": "text", "text": ""}]}]"#)]
#[case::other_kind(r#""artifacts": [{"parts": [{"kind": "data", "text": "hidden"}]}]"#)]
#[case::no_parts(r#""artifacts": [{}]"#)]
fn extract_skips_artifacts_without_text(#[case] artifacts: &str) {
    let extracted = extract(&format!(
        r#"{{{artifacts}, "history": [{{"role": "agent", "messageId": "m1", "parts": [{{"kind": "text", "text": "history"}}]}}]}}"#
    ));
    assert_eq!(extracted.text, "history");
    assert_eq!(extracted.message_id, "m1");
}

#[test]
fn extract_takes_the_last_agent_message_of_the_history() {
    let extracted = extract(
        r#"{"history": [
            {"role": "agent", "messageId": "old", "parts": [{"kind": "text", "text": "old"}]},
            {"role": "agent", "messageId": "new", "parts": [{"kind": "text", "text": "new"}]},
            {"role": "user", "messageId": "user", "parts": [{"kind": "text", "text": "question"}]}
        ]}"#,
    );
    assert_eq!(extracted.text, "new");
    assert_eq!(extracted.message_id, "new");
}

#[test]
fn extract_skips_agent_messages_without_text() {
    let extracted = extract(
        r#"{"history": [
            {"role": "agent", "messageId": "old", "parts": [{"kind": "text", "text": "old"}]},
            {"role": "agent", "messageId": "tool", "parts": [{"kind": "data", "data": {"a": 1}}]},
            {"role": "agent", "messageId": "blank", "parts": [{"kind": "text", "text": ""}]}
        ]}"#,
    );
    assert_eq!(extracted.text, "old");
    assert_eq!(extracted.message_id, "old");
}

#[test]
fn extract_concatenates_history_text_parts_without_a_separator() {
    let extracted = extract(
        r#"{"history": [{"role": "agent", "parts": [
            {"kind": "text", "text": "one"},
            {"kind": "data", "data": {}},
            {"kind": "text", "text": "two"}
        ]}]}"#,
    );
    assert_eq!(extracted.text, "onetwo");
    assert_eq!(extracted.parts.len(), 3);
}

#[test]
fn extract_generates_a_message_id_when_the_history_message_has_none() {
    let extracted =
        extract(r#"{"history": [{"role": "agent", "parts": [{"kind": "text", "text": "x"}]}]}"#);
    assert_eq!(extracted.message_id, uuid(1));
}

#[test]
fn extract_falls_back_to_the_message_field() {
    let extracted = extract(
        r#"{
            "history": [{"role": "user", "parts": [{"kind": "text", "text": "question"}]}],
            "message": {"messageId": "m", "parts": [{"kind": "text", "text": "a"}, {"kind": "text", "text": "b"}]}
        }"#,
    );
    assert_eq!(extracted.text, "ab");
    assert_eq!(extracted.message_id, "m");
    assert_eq!(extracted.parts.len(), 2);
}

#[test]
fn extract_message_field_keeps_parts_without_text() {
    let extracted = extract(r#"{"message": {"parts": [{"kind": "data", "data": {}}]}}"#);
    assert_eq!(extracted.text, "");
    assert_eq!(extracted.message_id, uuid(1));
    assert_eq!(extracted.parts.len(), 1);
}

#[rstest]
#[case::empty_result("{}")]
#[case::null_fields(r#"{"artifacts": null, "history": null, "message": null}"#)]
#[case::empty_message(r#"{"message": {}}"#)]
fn extract_without_any_text_gives_a_fresh_id_and_no_parts(#[case] result: &str) {
    assert_eq!(
        extract(result),
        ResponseText {
            text: String::new(),
            message_id: uuid(1),
            parts: Vec::new(),
        }
    );
}

#[test]
fn extract_generates_a_fresh_message_id_per_call() {
    let task = response(r#"{"artifacts": [{"parts": [{"kind": "text", "text": "x"}]}]}"#);
    let mut source = Source::default();
    let first = extract_response_text(&task, &mut source);
    let second = extract_response_text(&task, &mut source);
    assert_eq!([first.message_id, second.message_id], [uuid(1), uuid(2)]);
}

#[test]
fn extract_rejects_a_non_object_artifact() {
    let error =
        serde_json::from_str::<TaskResponse>(r#"{"result": {"artifacts": ["text"]}}"#).unwrap_err();
    assert!(error.to_string().contains("invalid type"));
}
