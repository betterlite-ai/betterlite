#![cfg(test)]

use super::{completion_body, prompt_body};
use betterlite_models::decisions::Json as Plain;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_prompts::arguments::json::Json as Prompted;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

#[test]
fn converts_nested_completion_json_values() {
    let source: Options = serde_json::from_str(
        r#"{
            "null": null,
            "bool": true,
            "number": 2,
            "text": "hi",
            "list": [false, {"nested": "value"}],
            "object": {"key": "value"}
        }"#,
    )
    .expect("a JSON object");
    let converted = completion_body(&source).expect("a convertible body");
    assert_eq!(converted.get("null"), Some(&Plain::Null));
    assert_eq!(converted.get("bool"), Some(&Plain::Bool(true)));
    assert_eq!(
        converted.get("number"),
        Some(&Plain::Number(Number::from(2_u8)))
    );
    assert_eq!(
        converted.get("text"),
        Some(&Plain::Text(String::from("hi")))
    );
    assert_eq!(
        converted.get("list"),
        Some(&Plain::List(vec![
            Plain::Bool(false),
            Plain::Object(IndexMap::from([(
                String::from("nested"),
                Plain::Text(String::from("value"))
            )])),
        ]))
    );
    assert_eq!(
        converted.get("object"),
        Some(&Plain::Object(IndexMap::from([(
            String::from("key"),
            Plain::Text(String::from("value")),
        )])))
    );
}

#[rstest]
#[case::u128("123456789012345678901234567890")]
#[case::beyond_u128("12345678901234567890123456789012345678901234567890")]
#[case::negative("-123456789012345678901234567890")]
#[case::precise_fraction("0.123456789012345678901234567890")]
fn completion_body_keeps_a_number_that_fits_no_primitive(#[case] number: &str) {
    let source: Options = serde_json::from_str(&format!(
        r#"{{"big": {number}, "nested": [{{"number": {number}}}]}}"#
    ))
    .expect("a JSON object");
    let converted = completion_body(&source).expect("a convertible body");
    let big: Number = serde_json::from_str(number).expect("an exact number");
    assert_eq!(converted.get("big"), Some(&Plain::Number(big.clone())));
    assert_eq!(
        converted.get("nested"),
        Some(&Plain::List(vec![Plain::Object(IndexMap::from([(
            String::from("number"),
            Plain::Number(big),
        )]))]))
    );
}

#[test]
fn converts_all_prompt_json_variants() {
    let source = IndexMap::from([
        (String::from("null"), Prompted::Null),
        (String::from("bool"), Prompted::Bool(false)),
        (String::from("number"), Prompted::Number(Number::from(4_u8))),
        (String::from("text"), Prompted::Text(String::from("prompt"))),
        (
            String::from("list"),
            Prompted::List(vec![Prompted::Text(String::from("item"))]),
        ),
        (
            String::from("object"),
            Prompted::Object(IndexMap::from([(
                String::from("inner"),
                Prompted::Bool(true),
            )])),
        ),
    ]);
    let converted = prompt_body(&source);
    assert_eq!(converted.get("null"), Some(&Plain::Null));
    assert_eq!(converted.get("bool"), Some(&Plain::Bool(false)));
    assert_eq!(
        converted.get("number"),
        Some(&Plain::Number(Number::from(4_u8)))
    );
    assert_eq!(
        converted.get("text"),
        Some(&Plain::Text(String::from("prompt")))
    );
    assert_eq!(
        converted.get("list"),
        Some(&Plain::List(vec![Plain::Text(String::from("item"))]))
    );
    assert_eq!(
        converted.get("object"),
        Some(&Plain::Object(IndexMap::from([(
            String::from("inner"),
            Plain::Bool(true),
        )])))
    );
}
