use std::collections::BTreeMap;

use betterlite_interactions::types::json::Object as InteractionObject;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_shapes::{GenericResponseOutputItem, OutputText};
use jiff::tz::{Offset, TimeZone};
use rstest::rstest;

use super::parsed;
use crate::api::responses_transform::error::ResponsesTransformError;
use crate::api::responses_transform::response::to_interactions_response;

const CREATED: &str = "1700000000";

fn object(source: &str) -> InteractionObject {
    serde_json::from_str(source).expect("the fixture should parse")
}

fn zone(seconds: i32) -> TimeZone {
    TimeZone::fixed(Offset::from_seconds(seconds).expect("a valid offset"))
}

const MESSAGE: &str = r#"{
    "id": "resp_1", "created_at": 1700000000, "model": "resp-model", "status": "completed",
    "output": [{"type": "message", "id": "msg_1", "status": "completed", "role": "assistant",
        "content": [
            {"type": "output_text", "text": "hello", "annotations": []},
            {"type": "output_text", "text": " world", "annotations": []}
        ]}],
    "usage": {"input_tokens": 7, "output_tokens": 3, "total_tokens": 10}
}"#;

#[test]
fn builds_the_interaction_envelope_from_a_message() {
    let interaction = to_interactions_response(&parsed(MESSAGE), Some("gpt-4o"), &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(interaction.id.as_deref(), Some("resp_1"));
    assert_eq!(interaction.object.as_deref(), Some("interaction"));
    assert_eq!(interaction.model.as_deref(), Some("gpt-4o"));
    assert_eq!(interaction.agent, None);
    assert_eq!(interaction.status.as_deref(), Some("completed"));
    assert_eq!(interaction.created.as_deref(), Some("2023-11-14T22:13:20"));
    assert_eq!(interaction.updated, interaction.created);
    assert_eq!(
        interaction.usage,
        Some(object(
            r#"{"total_input_tokens": 7, "total_output_tokens": 3}"#
        ))
    );
    assert!(interaction.extra.is_empty());
    assert!(interaction.hidden_params.is_empty());
}

#[test]
fn a_message_fills_the_legacy_outputs_and_one_step() {
    let interaction = to_interactions_response(&parsed(MESSAGE), Some("m"), &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(
        interaction.outputs,
        Some(vec![
            object(r#"{"type": "text", "text": "hello"}"#),
            object(r#"{"type": "text", "text": " world"}"#),
        ])
    );
    assert_eq!(
        interaction.steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [
                {"type": "text", "text": "hello"},
                {"type": "text", "text": " world"}
            ]}"#
        )])
    );
}

#[test]
fn each_output_item_with_text_becomes_its_own_step() {
    let interaction = to_interactions_response(
        &parsed(
            r#"{"id": "r", "created_at": 1, "output": [
                {"type": "message", "id": "m1", "status": "completed", "role": "assistant",
                 "content": [{"type": "output_text", "text": "a", "annotations": []}]},
                {"type": "message", "id": "m2", "status": "completed", "role": "assistant",
                 "content": [{"type": "output_text", "text": "b", "annotations": []}]}
            ]}"#,
        ),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(
        interaction.outputs,
        Some(vec![
            object(r#"{"type": "text", "text": "a"}"#),
            object(r#"{"type": "text", "text": "b"}"#),
        ])
    );
    assert_eq!(
        interaction.steps,
        Some(vec![
            object(r#"{"type": "model_output", "content": [{"type": "text", "text": "a"}]}"#),
            object(r#"{"type": "model_output", "content": [{"type": "text", "text": "b"}]}"#),
        ])
    );
}

#[test]
fn an_empty_output_gives_empty_lists() {
    let interaction = to_interactions_response(
        &parsed(r#"{"id": "r", "created_at": 1, "output": []}"#),
        None,
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(interaction.outputs, Some(Vec::new()));
    assert_eq!(interaction.steps, Some(Vec::new()));
}

#[test]
fn items_without_text_add_nothing() {
    let interaction = to_interactions_response(
        &parsed(
            r#"{"id": "r", "created_at": 1, "output": [
                {"type": "function_call", "id": "fc_1", "call_id": "c1", "name": "f",
                 "arguments": "{}", "status": "completed"},
                {"type": "message", "id": "m", "status": "completed", "role": "assistant",
                 "content": [{"type": "refusal", "refusal": "no"}]},
                {"type": "future_item", "id": "x", "content": [{"type": "text", "text": "raw"}]}
            ]}"#,
        ),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(interaction.outputs, Some(Vec::new()));
    assert_eq!(interaction.steps, Some(Vec::new()));
}

#[test]
fn reasoning_content_counts_as_text() {
    let interaction = to_interactions_response(
        &parsed(
            r#"{"id": "r", "created_at": 1, "output": [
                {"type": "reasoning", "id": "rs_1",
                 "summary": [{"type": "summary_text", "text": "sum"}],
                 "content": [{"type": "reasoning_text", "text": "think"}]}
            ]}"#,
        ),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(
        interaction.outputs,
        Some(vec![object(r#"{"type": "text", "text": "think"}"#)])
    );
    assert_eq!(
        interaction.steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [{"type": "text", "text": "think"}]}"#
        )])
    );
}

#[test]
fn typed_message_parts_without_text_are_skipped() {
    let mut decoded = parsed(r#"{"id": "r", "created_at": 1, "output": []}"#);
    decoded.output = vec![ResponseOutputItem::Message(GenericResponseOutputItem {
        r#type: String::from("message"),
        id: String::from("m"),
        status: String::from("completed"),
        role: String::from("assistant"),
        content: vec![
            OutputText {
                r#type: Some(String::from("output_text")),
                text: Some(String::from("hi")),
                annotations: None,
                extra: BTreeMap::new(),
            },
            OutputText {
                r#type: Some(String::from("output_text")),
                text: None,
                annotations: None,
                extra: BTreeMap::new(),
            },
        ],
        phase: None,
        extra: BTreeMap::new(),
    })];
    let interaction = to_interactions_response(&decoded, Some("m"), &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(
        interaction.outputs,
        Some(vec![object(r#"{"type": "text", "text": "hi"}"#)])
    );
}

#[rstest]
#[case::completed("completed")]
#[case::in_progress("in_progress")]
#[case::incomplete("incomplete")]
#[case::future("future_status")]
fn statuses_pass_through(#[case] status: &str) {
    let interaction = to_interactions_response(
        &parsed(&format!(
            r#"{{"id": "r", "created_at": 1, "output": [], "status": "{status}"}}"#
        )),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(interaction.status.as_deref(), Some(status));
}

#[test]
fn a_missing_status_stays_none() {
    let interaction = to_interactions_response(
        &parsed(r#"{"id": "r", "created_at": 1, "output": []}"#),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(interaction.status, None);
}

#[test]
fn usage_maps_to_the_interaction_totals() {
    let interaction = to_interactions_response(
        &parsed(
            r#"{"id": "r", "created_at": 1, "output": [],
                "usage": {"input_tokens": 7, "output_tokens": 3, "total_tokens": 10}}"#,
        ),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(
        interaction.usage,
        Some(object(
            r#"{"total_input_tokens": 7, "total_output_tokens": 3}"#
        ))
    );
}

#[test]
fn zero_usage_is_kept() {
    let interaction = to_interactions_response(
        &parsed(
            r#"{"id": "r", "created_at": 1, "output": [],
                "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}}"#,
        ),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(
        interaction.usage,
        Some(object(
            r#"{"total_input_tokens": 0, "total_output_tokens": 0}"#
        ))
    );
}

#[test]
fn an_absent_usage_stays_none() {
    let interaction = to_interactions_response(
        &parsed(r#"{"id": "r", "created_at": 1, "output": []}"#),
        Some("m"),
        &TimeZone::UTC,
    )
    .expect("the response should transform");
    assert_eq!(interaction.usage, None);
}

#[test]
fn the_model_argument_wins_and_the_response_model_is_the_fallback() {
    let decoded = parsed(r#"{"id": "r", "created_at": 1, "output": [], "model": "resp-model"}"#);
    let named = to_interactions_response(&decoded, Some("gpt-4o"), &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(named.model.as_deref(), Some("gpt-4o"));
    let empty = to_interactions_response(&decoded, Some(""), &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(empty.model.as_deref(), Some("resp-model"));
    let absent = to_interactions_response(&decoded, None, &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(absent.model.as_deref(), Some("resp-model"));
    let bare = parsed(r#"{"id": "r", "created_at": 1, "output": []}"#);
    let missing = to_interactions_response(&bare, None, &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(missing.model, None);
}

#[test]
fn created_uses_the_injected_local_zone() {
    let decoded = parsed(&format!(
        r#"{{"id": "r", "created_at": {CREATED}, "output": []}}"#
    ));
    let utc = to_interactions_response(&decoded, Some("m"), &TimeZone::UTC)
        .expect("the response should transform");
    assert_eq!(utc.created.as_deref(), Some("2023-11-14T22:13:20"));
    let shifted = to_interactions_response(&decoded, Some("m"), &zone(3600))
        .expect("the response should transform");
    assert_eq!(shifted.created.as_deref(), Some("2023-11-14T23:13:20"));
    assert_eq!(shifted.updated, shifted.created);
}

#[test]
fn an_out_of_range_created_at_is_refused() {
    let decoded = parsed(r#"{"id": "r", "created_at": 9223372036854775807, "output": []}"#);
    let error = to_interactions_response(&decoded, Some("m"), &TimeZone::UTC)
        .expect_err("the timestamp is out of range");
    assert!(matches!(error, ResponsesTransformError::CreatedAt { .. }));
}
