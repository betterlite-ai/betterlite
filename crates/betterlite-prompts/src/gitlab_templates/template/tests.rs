#![cfg(test)]

use rstest::rstest;

use super::{GitLabPromptTemplate, TemplateError};
use crate::arguments::json::{Json, Object};

fn metadata(json_text: &str) -> Object {
    serde_json::from_str(json_text).unwrap()
}

fn template(json_text: &str, model: Option<&str>) -> Result<GitLabPromptTemplate, TemplateError> {
    GitLabPromptTemplate::new(
        "id".to_owned(),
        "body".to_owned(),
        metadata(json_text),
        model.map(str::to_owned),
    )
}

#[test]
fn model_temperature_and_max_tokens_come_from_the_metadata() {
    let built = template(
        r#"{"model": "gpt-4", "temperature": 0.5, "max_tokens": 100}"#,
        None,
    )
    .unwrap();
    assert_eq!(built.model.as_deref(), Some("gpt-4"));
    assert_eq!(built.temperature, serde_json::from_str("0.5").ok());
    assert_eq!(built.max_tokens, serde_json::from_str("100").ok());
    assert_eq!(built.template_id, "id");
    assert_eq!(built.content, "body");
}

#[test]
fn absent_and_null_values_are_none() {
    let built = template(r#"{"temperature": null}"#, None).unwrap();
    assert_eq!(built.model, None);
    assert_eq!(built.temperature, None);
    assert_eq!(built.max_tokens, None);
}

#[rstest]
#[case::argument_wins(Some("override"), "override")]
#[case::empty_argument_defers(Some(""), "gpt-4")]
#[case::no_argument(None, "gpt-4")]
fn the_model_argument_beats_the_metadata_model(
    #[case] argument: Option<&str>,
    #[case] expected: &str,
) {
    let built = template(r#"{"model": "gpt-4"}"#, argument).unwrap();
    assert_eq!(built.model.as_deref(), Some(expected));
}

#[test]
fn optional_params_hold_every_key_but_model_input_and_content() {
    let built = template(
        r#"{"model": "m", "input": {"schema": {}}, "content": "c", "temperature": 1, "top_p": 0.9, "stop": ["x"]}"#,
        None,
    )
    .unwrap();
    let keys: Vec<&str> = built.optional_params.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["temperature", "top_p", "stop"]);
    assert_eq!(built.metadata.len(), 6);
}

#[rstest]
#[case::declared(
    r#"{"input": {"schema": {"name": "string"}}}"#,
    r#"{"name": "string"}"#
)]
#[case::no_schema(r#"{"input": {}}"#, "{}")]
#[case::no_input("{}", "{}")]
#[case::null_input(r#"{"input": null}"#, "{}")]
fn the_input_schema_is_read_from_input_schema(#[case] metadata_json: &str, #[case] expected: &str) {
    let built = template(metadata_json, None).unwrap();
    assert_eq!(
        built.input_schema,
        serde_json::from_str::<Json>(expected).unwrap()
    );
}

#[test]
fn a_model_that_is_not_text_is_an_error() {
    let error = template(r#"{"model": 4}"#, None).unwrap_err();
    assert_eq!(error.to_string(), "metadata field 'model' must be a string");
}

#[test]
fn a_model_argument_skips_the_metadata_model_check() {
    template(r#"{"model": 4}"#, Some("m")).unwrap();
}

#[test]
fn an_input_that_is_not_a_mapping_is_an_error() {
    let error = template(r#"{"input": ["a"]}"#, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "metadata field 'input' must be a mapping"
    );
}
