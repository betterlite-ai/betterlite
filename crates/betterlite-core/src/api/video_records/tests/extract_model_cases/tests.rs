#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::video_records::helpers::extract_model_from_target_model_names;

use super::fixture::assert_not_text;

#[test]
fn extract_model_from_text_inputs() {
    let cases = [
        ("openai/gpt-4o", Some("openai/gpt-4o")),
        ("single", Some("single")),
        ("a,b", Some("a")),
        ("  a-model , b-model  ", Some("a-model")),
        ("  foo  ", Some("foo")),
        ("", None),
        ("   ", None),
        (",,", None),
        (" , , ", None),
    ];
    for (input, expected) in cases {
        assert_eq!(
            extract_model_from_target_model_names(&Json::Text(input.to_owned())).unwrap(),
            expected.map(str::to_owned),
            "{input:?}"
        );
    }
}

#[test]
fn extract_model_from_list_inputs() {
    let cases: [(&[&str], Option<&str>); 4] = [
        (&[], None),
        (&["first", "second"], Some("first")),
        (&[" padded "], Some(" padded ")),
        (&["", "foo"], Some("")),
    ];
    for (names, expected) in cases {
        let input = Json::List(
            names
                .iter()
                .map(|name| Json::Text((*name).to_owned()))
                .collect(),
        );
        assert_eq!(
            extract_model_from_target_model_names(&input).unwrap(),
            expected.map(str::to_owned),
            "{names:?}"
        );
    }
}

#[test]
fn extract_model_from_list_with_a_null_first_entry() {
    let input = Json::List(vec![Json::Null]);
    assert_eq!(extract_model_from_target_model_names(&input).unwrap(), None);
}

#[test]
fn extract_model_returns_none_for_other_types() {
    let cases = [
        Json::Null,
        Json::Bool(true),
        Json::Bool(false),
        Json::Number(Number::from(12_i64)),
        Json::Object(IndexMap::new()),
    ];
    for input in cases {
        assert_eq!(
            extract_model_from_target_model_names(&input).unwrap(),
            None,
            "{input:?}"
        );
    }
}

#[test]
fn extract_model_rejects_a_non_text_list_entry() {
    let cases = [
        Json::Bool(true),
        Json::Number(Number::from(5_i64)),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ];
    for entry in cases {
        let error = extract_model_from_target_model_names(&Json::List(vec![entry])).unwrap_err();
        assert_not_text(error, "target_model_names");
    }
}
