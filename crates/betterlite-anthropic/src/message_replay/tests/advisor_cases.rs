use betterlite_prompts::arguments::json::Json;

use super::case_fixtures::ADVISOR_INPUT;
use super::{list_fixture, object_fixture};
use crate::message_replay::advisor::strip_advisor_blocks;

fn fixture_messages() -> Vec<Json> {
    match object_fixture(ADVISOR_INPUT).get("messages") {
        Some(Json::List(messages)) => messages.clone(),
        _ => panic!("the advisor fixture must carry messages"),
    }
}

#[test]
fn advisor_strip_mode_removes_uses_and_results_without_feedback() {
    let mut messages = fixture_messages();
    strip_advisor_blocks(&mut messages, false);
    let expected = list_fixture(
        r#"["pass-through",{"role":"user","content":[{"type":"server_tool_use","id":"adv_u","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv_u","content":"user advice"}]},{"role":"assistant","content":[{"type":"text","text":"before"},{"type":"text","text":"after"}]},{"role":"assistant","content":[{"type":"advisor_tool_result","tool_use_id":"orphan","content":"orphan advice"},{"type":"text","text":"kept"}]},{"role":"assistant","content":"plain string"},{"role":"assistant","content":[]},{"role":"assistant","content":[]},{"role":"assistant","content":[{"type":"server_tool_use","id":"","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"","content":"empty id advice"},"opaque"]},{"role":"assistant","content":[{"type":"server_tool_use","id":"adv_5","name":"other","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv_5","content":"other tool"}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_advice_is_collected_per_message() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv_x","name":"advisor","input":{}}]},{"role":"assistant","content":[{"type":"advisor_tool_result","tool_use_id":"adv_x","content":"other turn"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let expected = list_fixture(
        r#"[{"role":"assistant","content":[]},{"role":"assistant","content":[{"type":"advisor_tool_result","tool_use_id":"adv_x","content":"other turn"}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_matches_truthy_non_string_ids() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":7,"name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":7,"content":"numeric advice"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let expected = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"<advisor_feedback>\nnumeric advice\n</advisor_feedback>"}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_handles_distinct_ids_in_one_message() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv_a","name":"advisor","input":{}},{"type":"server_tool_use","id":"adv_b","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv_a","content":"advice a"},{"type":"advisor_tool_result","tool_use_id":"adv_b","content":"advice b"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let expected = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"<advisor_feedback>\nadvice a\n</advisor_feedback>"},{"type":"text","text":"<advisor_feedback>\nadvice b\n</advisor_feedback>"}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_only_touches_advisor_uses_and_results() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"tool_use","id":"adv","name":"lookup","input":{}},{"type":"other","tool_use_id":"adv","payload":1},{"type":"advisor_tool_result","tool_use_id":"adv","content":"advice"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let expected = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"<advisor_feedback>\nadvice\n</advisor_feedback>"},{"type":"tool_use","id":"adv","name":"lookup","input":{}},{"type":"other","tool_use_id":"adv","payload":1}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_replaces_every_use_of_a_repeated_id() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv","content":"once"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let expected = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"<advisor_feedback>\nonce\n</advisor_feedback>"},{"type":"text","text":"<advisor_feedback>\nonce\n</advisor_feedback>"}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_result_without_content_yields_no_feedback_block() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    assert_eq!(
        messages,
        list_fixture(r#"[{"role":"assistant","content":[]}]"#)
    );
}

#[test]
fn advisor_result_without_text_block_yields_no_feedback_block() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv","content":[{"type":"tool_use","id":"other"}]}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    assert_eq!(
        messages,
        list_fixture(r#"[{"role":"assistant","content":[]}]"#)
    );
}

// LiteLLM raises TypeError on this content shape; see docs/deviations/B571.yaml.
#[test]
fn advisor_non_iterable_result_content_yields_no_feedback_block() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv","content":5}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    assert_eq!(
        messages,
        list_fixture(r#"[{"role":"assistant","content":[]}]"#)
    );
}

// LiteLLM formats a non-string text value with an f-string; see docs/deviations/B571.yaml.
#[test]
fn advisor_non_string_text_value_yields_no_feedback_block() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv","content":[{"type":"text","text":5}]}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    assert_eq!(
        messages,
        list_fixture(r#"[{"role":"assistant","content":[]}]"#)
    );
}

#[test]
fn advisor_ignores_messages_without_assistant_role_or_list_content() {
    let mut messages = list_fixture(
        r#"[{"role":"user","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}}]},{"role":"assistant","content":"text"},{"role":"assistant"},{"content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}}]}]"#,
    );
    let original = messages.clone();
    strip_advisor_blocks(&mut messages, true);
    assert_eq!(messages, original);
}

#[test]
fn advisor_leaves_unmatched_blocks_in_order() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"text","text":"b"},{"type":"advisor_tool_result","tool_use_id":"adv","content":"advice"},{"type":"text","text":"c"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let expected = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"<advisor_feedback>\nadvice\n</advisor_feedback>"},{"type":"text","text":"b"},{"type":"text","text":"c"}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn advisor_feedback_block_has_only_type_and_text() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv","content":"advice"}]}]"#,
    );
    strip_advisor_blocks(&mut messages, true);
    let content = messages
        .first()
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("content"));
    let Some(Json::List(blocks)) = content else {
        panic!("the message content must be a list");
    };
    let Some(Json::Object(feedback)) = blocks.first() else {
        panic!("the feedback block must be an object");
    };
    let keys: Vec<&str> = feedback.keys().map(String::as_str).collect();
    assert_eq!(keys, ["type", "text"]);
    assert_eq!(
        feedback.get("text"),
        Some(&Json::Text(
            "<advisor_feedback>\nadvice\n</advisor_feedback>".to_owned()
        ))
    );
}

#[test]
fn advisor_keeps_message_fields_and_key_order() {
    let mut messages = list_fixture(
        r#"[{"role":"assistant","metadata":"m","content":[{"type":"server_tool_use","id":"adv","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"adv","content":"advice"}],"other":1}]"#,
    );
    strip_advisor_blocks(&mut messages, false);
    let fields = messages.first().and_then(Json::as_object).unwrap();
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(keys, ["role", "metadata", "content", "other"]);
    assert_eq!(fields.get("metadata"), Some(&Json::Text("m".to_owned())));
    assert_eq!(
        fields.get("other"),
        Some(&Json::Number(serde_json::Number::from(1_u64)))
    );
}
