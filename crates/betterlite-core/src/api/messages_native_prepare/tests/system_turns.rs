use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::support::{json as json_value, object};
use crate::api::messages_native_prepare::system_turns::{
    as_system_content_blocks, convert_mid_system_turns, normalize_system_turns,
    system_role_message_as_user,
};

const NOTE: &str = "Operator note (not from the user): the following was originally a \
                    mid-conversation system-role reminder.";

fn text_block(text: &str) -> Json {
    Json::Object(Object::from([
        ("type".to_owned(), Json::Text("text".to_owned())),
        ("text".to_owned(), Json::Text(text.to_owned())),
    ]))
}

fn reminder(text: &str) -> Json {
    Json::Object(Object::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        (
            "content".to_owned(),
            Json::List(vec![text_block(NOTE), text_block(text)]),
        ),
    ]))
}

fn message(text: &str) -> Json {
    Json::Object(object(text))
}

fn normalized(request: &str, supports_mid_system: bool) -> Object {
    let mut given = object(request);
    normalize_system_turns(&mut given, supports_mid_system);
    given
}

const TOOL_TURNS: &str = r#"{"model":"claude-sonnet-4-5","max_tokens":16,"system":"base","messages":[
    {"role":"system","content":"initial"},
    {"role":"user","content":"question"},
    {"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{}}]},
    {"role":"system","content":"reminder"},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"found"}]},
    {"role":"user","content":"next"}]}"#;

fn tool_turns_head(tail: Vec<Json>) -> Object {
    Object::from([
        (
            "model".to_owned(),
            Json::Text("claude-sonnet-4-5".to_owned()),
        ),
        ("max_tokens".to_owned(), json_value("16")),
        (
            "system".to_owned(),
            Json::List(vec![text_block("base"), text_block("initial")]),
        ),
        ("messages".to_owned(), Json::List(tail)),
    ])
}

#[test]
fn system_turn_placement_moves_a_reminder_behind_its_tool_result() {
    let expected = tool_turns_head(vec![
        message(r#"{"role":"user","content":"question"}"#),
        message(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{}}]}"#,
        ),
        message(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"found"}]}"#,
        ),
        reminder("reminder"),
        message(r#"{"role":"user","content":"next"}"#),
    ]);
    assert_eq!(normalized(TOOL_TURNS, false), expected);
}

#[test]
fn system_turn_placement_keeps_a_flagged_mid_conversation_turn() {
    let expected = tool_turns_head(vec![
        message(r#"{"role":"user","content":"question"}"#),
        message(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"lookup","input":{}}]}"#,
        ),
        message(r#"{"role":"system","content":"reminder"}"#),
        message(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"found"}]}"#,
        ),
        message(r#"{"role":"user","content":"next"}"#),
    ]);
    assert_eq!(normalized(TOOL_TURNS, true), expected);
}

#[rstest]
#[case::missing_messages(r#"{"system":"base"}"#)]
#[case::non_list_messages(r#"{"system":"base","messages":"not a list"}"#)]
fn system_turn_placement_leaves_an_unusable_message_list_alone(#[case] request: &str) {
    assert_eq!(normalized(request, false), object(request));
    assert_eq!(normalized(request, true), object(request));
}

#[test]
fn system_turn_placement_rebuilds_an_empty_message_list() {
    let expected = object(r#"{"system":[{"type":"text","text":"base"}],"messages":[]}"#);
    assert_eq!(
        normalized(r#"{"system":"base","messages":[]}"#, false),
        expected
    );
    assert_eq!(
        normalized(r#"{"system":"base","messages":[]}"#, true),
        expected
    );
}

#[test]
fn system_turn_placement_hoists_an_all_system_list() {
    let request = r#"{"messages":[{"role":"system","content":"a"},{"role":"system","content":[{"type":"text","text":"b"}]}]}"#;
    let expected = Object::from([
        ("messages".to_owned(), Json::List(Vec::new())),
        (
            "system".to_owned(),
            Json::List(vec![text_block("a"), text_block("b")]),
        ),
    ]);
    assert_eq!(normalized(request, false), expected);
}

#[test]
fn system_turn_placement_converts_a_trailing_run() {
    let request =
        r#"{"messages":[{"role":"user","content":"q"},{"role":"system","content":"tail"}]}"#;
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![
            message(r#"{"role":"user","content":"q"}"#),
            reminder("tail"),
        ]),
    )]);
    assert_eq!(normalized(request, false), expected);
}

#[test]
fn system_turn_placement_converts_every_reminder_in_place() {
    let request = r#"{"messages":[
        {"role":"user","content":"q"},
        {"role":"system","content":"first"},
        {"role":"user","content":"u1"},
        {"role":"system","content":"second"},
        {"role":"user","content":"u2"}]}"#;
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![
            message(r#"{"role":"user","content":"q"}"#),
            reminder("first"),
            message(r#"{"role":"user","content":"u1"}"#),
            reminder("second"),
            message(r#"{"role":"user","content":"u2"}"#),
        ]),
    )]);
    assert_eq!(normalized(request, false), expected);
}

#[test]
fn system_turn_placement_leaves_a_run_without_a_tool_result_first_alone() {
    let request = r#"{"messages":[
        {"role":"user","content":"q"},
        {"role":"assistant","content":"a"},
        {"role":"system","content":"r"},
        {"role":"user","content":[{"type":"text","text":"look"},{"type":"tool_result","tool_use_id":"t"}]}]}"#;
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![
            message(r#"{"role":"user","content":"q"}"#),
            message(r#"{"role":"assistant","content":"a"}"#),
            reminder("r"),
            message(
                r#"{"role":"user","content":[{"type":"text","text":"look"},{"type":"tool_result","tool_use_id":"t"}]}"#,
            ),
        ]),
    )]);
    assert_eq!(normalized(request, false), expected);
}

#[test]
fn system_turn_placement_filters_billing_text_without_hoisting_anything() {
    let request = r#"{"system":[{"type":"text","text":"x-anthropic-billing-header: a"}],"messages":[{"role":"user","content":"q"}]}"#;
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![message(r#"{"role":"user","content":"q"}"#)]),
    )]);
    assert_eq!(normalized(request, false), expected);
}

#[test]
fn system_turn_placement_carries_only_the_converted_content() {
    let request = r#"{"messages":[
        {"role":"user","content":"q"},
        {"role":"system","content":"r","name":"ops","cache_control":{"type":"ephemeral"}}]}"#;
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![
            message(r#"{"role":"user","content":"q"}"#),
            reminder("r"),
        ]),
    )]);
    assert_eq!(normalized(request, false), expected);
}

#[test]
fn system_content_blocks_map_every_system_shape() {
    let text = text_block("x");
    assert_eq!(as_system_content_blocks(None), Vec::new());
    assert_eq!(
        as_system_content_blocks(Some(&json_value("null"))),
        Vec::new()
    );
    assert_eq!(
        as_system_content_blocks(Some(&json_value(r#""x""#))),
        vec![text.clone()]
    );
    assert_eq!(
        as_system_content_blocks(Some(&json_value(r#"[{"type":"text","text":"x"}]"#))),
        vec![text]
    );
    assert_eq!(
        as_system_content_blocks(Some(&json_value("5"))),
        vec![json_value("5")]
    );
}

#[test]
fn system_role_messages_convert_with_the_operator_note() {
    let converted = system_role_message_as_user(&object(
        r#"{"role":"system","content":"x","cache_control":{"type":"ephemeral"}}"#,
    ));
    assert_eq!(Json::Object(converted), reminder("x"));
}

#[test]
fn convert_mid_system_turns_keeps_non_system_messages_byte_identical() {
    let messages = vec![
        json_value(r#"{"role":"user","content":"q"}"#),
        json_value(r#"{"role":"assistant","content":"a","tool_calls":[]}"#),
    ];
    assert_eq!(convert_mid_system_turns(&messages), messages);
}
