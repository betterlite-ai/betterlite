use super::*;

use crate::model_listing::response::{PRESERVED_NONE_FIELDS, model_dump_with_preserved_fields};

fn choice(sub_object: &str, fields: Vec<(&str, Json)>) -> Json {
    Json::Object(object(vec![(sub_object, Json::Object(object(fields)))]))
}

fn dump(choices: Vec<Json>) -> JsonObject {
    object(vec![("choices", Json::List(choices))])
}

#[test]
fn the_preserved_none_fields_are_the_three_openai_requires() {
    assert_eq!(
        PRESERVED_NONE_FIELDS,
        [
            ("message", "content"),
            ("message", "role"),
            ("delta", "content")
        ]
    );
}

#[test]
fn a_stripped_message_field_is_restored_from_the_original_choice() {
    let dumped = dump(vec![choice("message", Vec::new())]);
    let original = vec![object(vec![(
        "message",
        Json::Object(object(vec![
            ("content", Json::Null),
            ("role", text("assistant")),
        ])),
    )])];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(
        restored,
        dump(vec![choice(
            "message",
            vec![("content", Json::Null), ("role", text("assistant"))]
        )])
    );
}

#[test]
fn a_stripped_delta_content_is_restored() {
    let dumped = dump(vec![choice("delta", Vec::new())]);
    let original = vec![object(vec![(
        "delta",
        Json::Object(object(vec![("content", Json::Null)])),
    )])];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(
        restored,
        dump(vec![choice("delta", vec![("content", Json::Null)])])
    );
}

#[test]
fn a_field_the_dump_still_carries_keeps_its_value() {
    let dumped = dump(vec![choice("message", vec![("content", text("hi"))])]);
    let original = vec![object(vec![(
        "message",
        Json::Object(object(vec![("content", Json::Null)])),
    )])];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(
        restored,
        dump(vec![choice("message", vec![("content", text("hi"))])])
    );
}

#[test]
fn a_choice_without_the_sub_object_is_left_alone() {
    let dumped = dump(vec![Json::Object(object(vec![("index", number(0))]))]);
    let original = vec![object(vec![(
        "message",
        Json::Object(object(vec![("content", Json::Null)])),
    )])];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(
        restored,
        dump(vec![Json::Object(object(vec![("index", number(0))]))])
    );
}

#[test]
fn an_original_choice_without_the_sub_object_restores_nothing() {
    let dumped = dump(vec![choice("message", Vec::new())]);
    let original = vec![Json::Object(JsonObject::new())];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(restored, dump(vec![choice("message", Vec::new())]));
}

#[test]
fn an_original_sub_object_without_the_field_restores_nothing() {
    let dumped = dump(vec![choice("message", Vec::new())]);
    let original = vec![object(vec![(
        "message",
        Json::Object(object(vec![("tool_calls", Json::List(Vec::new()))])),
    )])];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(restored, dump(vec![choice("message", Vec::new())]));
}

#[test]
fn a_dump_without_choices_is_returned_unchanged() {
    let restored = model_dump_with_preserved_fields(
        object(vec![("id", text("cmp"))]),
        &[choice("message", Vec::new())],
    );
    assert_eq!(restored, object(vec![("id", text("cmp"))]));
}

#[test]
fn choices_that_are_not_a_list_are_returned_unchanged() {
    let dumped = object(vec![("choices", text("none"))]);
    let restored =
        model_dump_with_preserved_fields(dumped, &[object(vec![("message", Json::Null)])]);
    assert_eq!(restored, object(vec![("choices", text("none"))]));
}

#[test]
fn a_choice_that_is_not_an_object_is_skipped() {
    let dumped = dump(vec![text("odd"), choice("message", Vec::new())]);
    let original = vec![
        object(vec![(
            "message",
            Json::Object(object(vec![("role", text("assistant"))])),
        )]),
        object(vec![(
            "message",
            Json::Object(object(vec![("role", text("user"))])),
        )]),
    ];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(
        restored,
        dump(vec![
            text("odd"),
            choice("message", vec![("role", text("user"))])
        ])
    );
}

#[test]
fn choices_past_the_original_list_are_left_alone() {
    let dumped = dump(vec![
        choice("message", Vec::new()),
        choice("message", Vec::new()),
    ]);
    let original = vec![object(vec![(
        "message",
        Json::Object(object(vec![("role", text("assistant"))])),
    )])];
    let restored = model_dump_with_preserved_fields(dumped, &original);
    assert_eq!(
        restored,
        dump(vec![
            choice("message", vec![("role", text("assistant"))]),
            choice("message", Vec::new())
        ])
    );
}
