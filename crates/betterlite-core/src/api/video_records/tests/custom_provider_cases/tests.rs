#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
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

#[rstest]
#[case::null(Json::Null)]
#[case::bool(Json::Bool(true))]
#[case::number(Json::Number(Number::from(5_i64)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn custom_provider_ignores_a_non_text_extra_body_provider(#[case] value: Json) {
    let data = object(&[(
        "extra_body",
        Json::Object(object(&[("custom_llm_provider", value)])),
    )]);
    assert_eq!(get_custom_provider_from_data(&data).unwrap(), None);
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

#[rstest]
#[case::provider(r#"{"custom_llm_provider": "openai"}"#, Some("openai"))]
#[case::empty_provider(r#"{"custom_llm_provider": ""}"#, Some(""))]
#[case::number_provider(r#"{"custom_llm_provider": 7}"#, None)]
#[case::list(r#"["custom_llm_provider"]"#, None)]
#[case::null("null", None)]
#[case::empty("", None)]
#[case::not_json("not json", None)]
fn custom_provider_decodes_a_json_string_extra_body(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let data = object(&[("extra_body", Json::Text(text.to_owned()))]);
    assert_eq!(
        get_custom_provider_from_data(&data).unwrap(),
        expected.map(str::to_owned),
        "{text:?}"
    );
}

#[rstest]
#[case::empty_data(IndexMap::<String, Json>::new())]
#[case::other_field(object(&[("other", Json::Text("x".to_owned()))]))]
#[case::null_provider(object(&[("custom_llm_provider", Json::Null)]))]
#[case::empty_provider(object(&[("custom_llm_provider", Json::Text(String::new()))]))]
#[case::null_extra_body(object(&[("extra_body", Json::Null)]))]
#[case::number_extra_body(object(&[("extra_body", Json::Number(Number::from(5_i64)))]))]
#[case::bool_extra_body(object(&[("extra_body", Json::Bool(true))]))]
#[case::list_extra_body(object(&[("extra_body", Json::List(Vec::new()))]))]
fn custom_provider_returns_none_without_the_field(#[case] data: IndexMap<String, Json>) {
    assert_eq!(get_custom_provider_from_data(&data).unwrap(), None);
}

#[test]
fn custom_provider_falls_through_an_empty_top_level_field() {
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
        Some("vertex_ai".to_owned())
    );
}

#[rstest]
#[case::bool(Json::Bool(true))]
#[case::number(Json::Number(Number::from(5_i64)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn custom_provider_rejects_a_non_text_top_level_field(#[case] value: Json) {
    let data = object(&[("custom_llm_provider", value)]);
    let error = get_custom_provider_from_data(&data).unwrap_err();
    assert_not_text(error, "custom_llm_provider");
}
