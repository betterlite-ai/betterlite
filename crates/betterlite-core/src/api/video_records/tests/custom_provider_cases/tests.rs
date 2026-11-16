#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::video_records::helpers::get_custom_provider_from_data;

use super::fixture::{assert_not_text, object};

#[test]
fn custom_provider_reads_the_top_level_field() {
    let data = object(&[
        ("custom_llm_provider", Json::Text("openai".to_owned())),
        (
            "extra_body",
            Json::Object(object(&[(
                "custom_llm_provider",
                Json::Text("vertex_ai".to_owned()),
            )])),
        ),
    ]);
    assert_eq!(
        get_custom_provider_from_data(&data).unwrap(),
        Some("openai".to_owned())
    );
}

#[test]
fn custom_provider_reads_a_dict_extra_body() {
    let data = object(&[(
        "extra_body",
        Json::Object(object(&[(
            "custom_llm_provider",
            Json::Text("vertex_ai".to_owned()),
        )])),
    )]);
    assert_eq!(
        get_custom_provider_from_data(&data).unwrap(),
        Some("vertex_ai".to_owned())
    );
}

#[test]
fn custom_provider_falls_through_a_null_top_level_field() {
    let data = object(&[
        ("custom_llm_provider", Json::Null),
        (
            "extra_body",
            Json::Object(object(&[(
                "custom_llm_provider",
                Json::Text("vertex_ai".to_owned()),
            )])),
        ),
    ]);
    assert_eq!(
        get_custom_provider_from_data(&data).unwrap(),
        Some("vertex_ai".to_owned())
    );
}

#[test]
fn custom_provider_ignores_a_non_text_extra_body_provider() {
    let cases = [
        Json::Null,
        Json::Bool(true),
        Json::Number(Number::from(5_i64)),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ];
    for value in cases {
        let data = object(&[(
            "extra_body",
            Json::Object(object(&[("custom_llm_provider", value)])),
        )]);
        assert_eq!(get_custom_provider_from_data(&data).unwrap(), None);
    }
}

#[test]
fn custom_provider_keeps_an_explicit_empty_extra_body_provider() {
    let data = object(&[(
        "extra_body",
        Json::Object(object(&[(
            "custom_llm_provider",
            Json::Text(String::new()),
        )])),
    )]);
    assert_eq!(
        get_custom_provider_from_data(&data).unwrap(),
        Some(String::new())
    );
}

#[test]
fn custom_provider_decodes_a_json_string_extra_body() {
    let cases = [
        (r#"{"custom_llm_provider": "openai"}"#, Some("openai")),
        (r#"{"custom_llm_provider": ""}"#, Some("")),
        (r#"{"custom_llm_provider": 7}"#, None),
        (r#"["custom_llm_provider"]"#, None),
        ("null", None),
        ("", None),
        ("not json", None),
    ];
    for (text, expected) in cases {
        let data = object(&[("extra_body", Json::Text(text.to_owned()))]);
        assert_eq!(
            get_custom_provider_from_data(&data).unwrap(),
            expected.map(str::to_owned),
            "{text:?}"
        );
    }
}

#[test]
fn custom_provider_returns_none_without_the_field() {
    let cases = [
        IndexMap::new(),
        object(&[("other", Json::Text("x".to_owned()))]),
        object(&[("custom_llm_provider", Json::Null)]),
        object(&[("extra_body", Json::Null)]),
        object(&[("extra_body", Json::Number(Number::from(5_i64)))]),
        object(&[("extra_body", Json::Bool(true))]),
        object(&[("extra_body", Json::List(Vec::new()))]),
    ];
    for data in cases {
        assert_eq!(get_custom_provider_from_data(&data).unwrap(), None);
    }
}

#[test]
fn custom_provider_keeps_an_explicit_empty_top_level_field() {
    let data = object(&[
        ("custom_llm_provider", Json::Text(String::new())),
        (
            "extra_body",
            Json::Object(object(&[(
                "custom_llm_provider",
                Json::Text("vertex_ai".to_owned()),
            )])),
        ),
    ]);
    assert_eq!(
        get_custom_provider_from_data(&data).unwrap(),
        Some(String::new())
    );
}

#[test]
fn custom_provider_rejects_a_non_text_top_level_field() {
    let cases = [
        Json::Bool(true),
        Json::Number(Number::from(5_i64)),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ];
    for value in cases {
        let data = object(&[("custom_llm_provider", value)]);
        let error = get_custom_provider_from_data(&data).unwrap_err();
        assert_not_text(error, "custom_llm_provider");
    }
}
