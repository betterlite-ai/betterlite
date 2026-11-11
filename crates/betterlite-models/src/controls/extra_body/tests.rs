#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::controls::extra_body::{MAX_EXTRA_BODY_BYTES, MAX_EXTRA_BODY_DEPTH, check_extra_body};
use crate::controls::json_value::JsonValue;

enum Scenario {
    Accepts(JsonValue),
    TooDeep(JsonValue),
    TooLarge(JsonValue),
}

fn nested(depth: usize) -> JsonValue {
    let mut value = JsonValue::Number(Number::from(1_u64));
    for _ in 0..depth {
        value = JsonValue::Array(vec![value]);
    }
    value
}

fn text_body(size: usize) -> JsonValue {
    JsonValue::Text("a".repeat(size))
}

fn realistic_body() -> JsonValue {
    let prompt = JsonValue::Object(IndexMap::from([
        ("name".to_owned(), JsonValue::Text("p".to_owned())),
        ("version".to_owned(), JsonValue::Number(Number::from(2_u64))),
    ]));
    let metadata = JsonValue::Object(IndexMap::from([("prompt".to_owned(), prompt)]));
    JsonValue::Object(IndexMap::from([
        ("metadata".to_owned(), metadata),
        (
            "temperature".to_owned(),
            JsonValue::Number(Number::from_f64(0.2).expect("a finite number")),
        ),
    ]))
}

#[rstest]
#[case::null_body_accepts(Scenario::Accepts(JsonValue::Null))]
#[case::json_body_is_unchanged(Scenario::Accepts(realistic_body()))]
#[case::depth_at_the_limit(Scenario::Accepts(nested(MAX_EXTRA_BODY_DEPTH)))]
#[case::depth_past_the_limit(Scenario::TooDeep(nested(MAX_EXTRA_BODY_DEPTH + 1)))]
#[case::size_at_the_limit(Scenario::Accepts(text_body(MAX_EXTRA_BODY_BYTES - 2)))]
#[case::size_past_the_limit(Scenario::TooLarge(text_body(MAX_EXTRA_BODY_BYTES - 1)))]
fn provider_controls_tests_behavior(#[case] scenario: Scenario) {
    assert_eq!(MAX_EXTRA_BODY_BYTES, 0x0010_0000);
    assert_eq!(MAX_EXTRA_BODY_DEPTH, 100);
    match scenario {
        Scenario::Accepts(body) => check_extra_body(&body).expect("the body is within its bounds"),
        Scenario::TooDeep(body) => {
            let error = check_extra_body(&body).expect_err("the body is nested too deeply");
            assert!(error.to_string().contains("nesting"));
        }
        Scenario::TooLarge(body) => {
            let error = check_extra_body(&body).expect_err("the body is over the size limit");
            assert!(error.to_string().contains("byte limit"));
        }
    }
}
