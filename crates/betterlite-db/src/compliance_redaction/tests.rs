#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{RedactionKeys, redact_match_and_regex_keys};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn key(name: &str) -> String {
    name.to_owned()
}

fn object(fields: Vec<(String, Json)>) -> Json {
    Json::Object(fields.into_iter().collect())
}

fn nested(depth: usize, leaf: Json) -> Json {
    let mut value = leaf;
    for _ in 0..depth {
        value = object(vec![(key("child"), value)]);
    }
    value
}

fn redacted(payload: &Json) -> Json {
    redact_match_and_regex_keys(payload, &RedactionKeys::default())
}

#[test]
fn redacts_the_match_and_regex_fields_with_the_marker() {
    let payload = object(vec![
        (key("match"), text("secret span")),
        (key("regex"), text("a.*b")),
        (key("other"), text("kept")),
    ]);
    let expected = object(vec![
        (key("match"), text("[REDACTED]")),
        (key("regex"), text("[REDACTED]")),
        (key("other"), text("kept")),
    ]);
    assert_eq!(redacted(&payload), expected);
}

#[test]
fn keeps_a_field_order_the_payload_had() {
    let payload = object(vec![
        (key("zulu"), text("kept")),
        (key("regex"), text("r")),
        (key("alpha"), text("kept")),
    ]);
    assert_eq!(
        serde_json::to_string(&redacted(&payload)).expect("a redacted payload serializes"),
        r#"{"zulu":"kept","regex":"[REDACTED]","alpha":"kept"}"#
    );
}

#[test]
fn redacts_match_and_regex_fields_nested_in_objects_and_lists() {
    let payload = object(vec![(
        key("guardrail_response"),
        Json::List(vec![
            object(vec![(key("match"), text("one"))]),
            object(vec![(
                key("details"),
                Json::List(vec![object(vec![(key("regex"), text("two"))])]),
            )]),
        ]),
    )]);
    let expected = object(vec![(
        key("guardrail_response"),
        Json::List(vec![
            object(vec![(key("match"), text("[REDACTED]"))]),
            object(vec![(
                key("details"),
                Json::List(vec![object(vec![(key("regex"), text("[REDACTED]"))])]),
            )]),
        ]),
    )]);
    assert_eq!(redacted(&payload), expected);
}

#[test]
fn redacts_a_configured_field_whose_value_is_not_text() {
    let payload = object(vec![
        (key("match"), object(vec![(key("span"), text("inner"))])),
        (key("regex"), Json::List(vec![text("a"), text("b")])),
        (key("match_count"), number(3)),
    ]);
    let expected = object(vec![
        (key("match"), text("[REDACTED]")),
        (key("regex"), text("[REDACTED]")),
        (key("match_count"), number(3)),
    ]);
    assert_eq!(redacted(&payload), expected);
}

#[test]
fn keeps_an_unnested_match_value_out_of_the_walk() {
    let payload = object(vec![
        (key("match"), Json::Null),
        (key("regex"), Json::Bool(false)),
        (key("other_match"), Json::Null),
    ]);
    let expected = object(vec![
        (key("match"), text("[REDACTED]")),
        (key("regex"), text("[REDACTED]")),
        (key("other_match"), Json::Null),
    ]);
    assert_eq!(redacted(&payload), expected);
}

#[test]
fn keeps_names_that_only_look_like_a_configured_one() {
    let payload = object(vec![
        (key("matches"), text("kept")),
        (key("Match"), text("kept")),
        (key("match_details"), text("kept")),
        (key("regexes"), text("kept")),
        (key("regular_expression"), text("kept")),
    ]);
    assert_eq!(redacted(&payload), payload);
}

#[test]
fn redacts_only_the_configured_names() {
    let keys = RedactionKeys {
        names: vec![key("keyword"), key("snippet")],
    };
    let payload = object(vec![
        (key("keyword"), text("secret")),
        (key("snippet"), text("secret")),
        (key("match"), text("kept")),
    ]);
    let expected = object(vec![
        (key("keyword"), text("[REDACTED]")),
        (key("snippet"), text("[REDACTED]")),
        (key("match"), text("kept")),
    ]);
    assert_eq!(redact_match_and_regex_keys(&payload, &keys), expected);
}

#[rstest]
#[case::null(Json::Null)]
#[case::boolean(Json::Bool(true))]
#[case::number(number(1))]
#[case::text(Json::Text(String::from("payload")))]
#[case::empty_list(Json::List(Vec::new()))]
#[case::empty_object(Json::Object(IndexMap::new()))]
fn returns_a_value_that_is_not_a_redaction_key_unchanged(#[case] payload: Json) {
    assert_eq!(redacted(&payload), payload);
}

#[test]
fn returns_a_copy_that_leaves_the_payload_alone() {
    let payload = object(vec![(key("match"), text("secret"))]);
    let output = redacted(&payload);
    assert_eq!(
        serde_json::to_string(&payload).expect("the payload serializes"),
        r#"{"match":"secret"}"#
    );
    assert_ne!(output, payload);
}

#[test]
fn redacts_a_payload_nested_two_thousand_levels_deep() {
    let payload = nested(2_000, object(vec![(key("match"), text("secret"))]));
    let expected = nested(2_000, object(vec![(key("match"), text("[REDACTED]"))]));
    assert_eq!(redacted(&payload), expected);
}

#[test]
fn serializes_the_default_keys_as_the_two_field_names() {
    assert_eq!(
        serde_json::to_string(&RedactionKeys::default()).expect("the keys serialize"),
        r#"{"names":["match","regex"]}"#
    );
}
