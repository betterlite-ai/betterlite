#![cfg(test)]

use super::{completion_body, plain, prompt_body};
use betterlite_models::decisions::Json as Plain;
use betterlite_prompts::arguments::json::Json as Prompted;
use indexmap::IndexMap;
use serde_json::{Number, json};

#[test]
fn converts_nested_completion_json_values() {
    let source: IndexMap<String, serde_json::Value> = serde_json::from_value(json!({
        "null": null,
        "bool": true,
        "number": 2,
        "text": "hi",
        "list": [false, {"nested": "value"}],
        "object": {"key": "value"}
    }))
    .expect("a JSON object");
    let converted = completion_body(&source);
    assert_eq!(converted.get("null"), Some(&Plain::Null));
    assert_eq!(converted.get("bool"), Some(&Plain::Bool(true)));
    assert_eq!(
        converted.get("number"),
        Some(&Plain::Number(Number::from(2)))
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

#[test]
fn converts_all_prompt_json_variants() {
    let source = IndexMap::from([
        (String::from("null"), Prompted::Null),
        (String::from("bool"), Prompted::Bool(false)),
        (String::from("number"), Prompted::Number(Number::from(4))),
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
        Some(&Plain::Number(Number::from(4)))
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

#[test]
fn converts_individual_nested_values() {
    assert_eq!(
        plain(&json!({"nested": [1, null]})),
        Plain::Object(IndexMap::from([(
            String::from("nested"),
            Plain::List(vec![Plain::Number(Number::from(1)), Plain::Null]),
        )]))
    );
}
