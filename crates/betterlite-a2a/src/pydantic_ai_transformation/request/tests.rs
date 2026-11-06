#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{ParamsError, build_send_params, remove_none_values};
use crate::completion_bridge::wire::{A2AMessage, MessageSendParams, Role};

fn json(text: &str) -> AgentJson {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::object_fields(r#"{"a": null, "b": 1}"#, r#"{"b": 1}"#)]
#[case::nested_objects(
    r#"{"a": {"b": null, "c": {"d": null, "e": 2}}}"#,
    r#"{"a": {"c": {"e": 2}}}"#
)]
#[case::list_items("[1, null, 2]", "[1, 2]")]
#[case::objects_in_lists(
    r#"{"a": [{"b": null, "c": 3}, null, [null, 4]]}"#,
    r#"{"a": [{"c": 3}, [4]]}"#
)]
#[case::falsy_values_stay(
    r#"{"a": 0, "b": "", "c": [], "d": {}, "e": false}"#,
    r#"{"a": 0, "b": "", "c": [], "d": {}, "e": false}"#
)]
#[case::scalar_stays(r#""text""#, r#""text""#)]
fn pydantic_ai_submit_removes_none_values_recursively(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(remove_none_values(json(input)), json(expected));
}

#[test]
fn pydantic_ai_submit_forces_message_kind_after_none_removal() {
    let params = MessageSendParams {
        message: A2AMessage {
            role: Role("user".to_owned()),
            parts: vec![json(r#"{"kind": "text", "text": "hi", "metadata": null}"#)],
            message_id: None,
            metadata: Some(AgentJson::Null),
            extra: BTreeMap::from([("kind".to_owned(), json(r#""other""#))]),
        },
        metadata: None,
        extra: BTreeMap::from([("configuration".to_owned(), json(r#"{"blocking": null}"#))]),
    };
    let built = build_send_params(&params).unwrap();
    assert_eq!(
        AgentJson::Object(built.into_iter().collect()),
        json(
            r#"{
                "message": {"kind": "message", "role": "user", "parts": [{"kind": "text", "text": "hi"}]},
                "configuration": {}
            }"#
        )
    );
}

#[test]
fn pydantic_ai_submit_leaves_params_without_a_message_alone() {
    let built = build_send_params(&json(r#"{"configuration": {"a": null}}"#)).unwrap();
    assert_eq!(
        built,
        BTreeMap::from([("configuration".to_owned(), json("{}"))])
    );
}

#[test]
fn pydantic_ai_submit_drops_a_null_message_without_adding_a_kind() {
    let built = build_send_params(&json(r#"{"message": null}"#)).unwrap();
    assert!(built.is_empty());
}

#[rstest]
#[case::list("[1]")]
#[case::text(r#""text""#)]
#[case::null("null")]
fn pydantic_ai_submit_rejects_params_that_are_not_an_object(#[case] params: &str) {
    assert!(matches!(
        build_send_params(&json(params)),
        Err(ParamsError::NotObject)
    ));
}

#[rstest]
#[case::text(r#"{"message": "hi"}"#)]
#[case::list(r#"{"message": []}"#)]
fn pydantic_ai_submit_rejects_a_message_that_is_not_an_object(#[case] params: &str) {
    assert!(matches!(
        build_send_params(&json(params)),
        Err(ParamsError::MessageNotObject)
    ));
}

#[test]
fn pydantic_ai_submit_reports_params_that_cannot_be_serialized() {
    assert!(matches!(
        build_send_params(&BTreeMap::from([((1_u8, 2_u8), 3_u8)])),
        Err(ParamsError::Serialize(_))
    ));
}
