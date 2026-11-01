#![cfg(test)]

use super::PromptTemplate;
use crate::arguments::json::Object;
use rstest::rstest;

fn metadata(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

#[rstest]
fn extracts_restricted_keys_and_leaves_the_rest_as_optional_params() {
    let metadata = metadata(
        r#"{
            "model": "gpt-4",
            "input": {"schema": {"name": "string"}},
            "output": {"format": "text", "schema": {"answer": "string"}},
            "temperature": 0.7
        }"#,
    );
    let template =
        PromptTemplate::new("hi".to_owned(), metadata, Some("greeting".to_owned())).unwrap();

    assert_eq!(template.content, "hi");
    assert_eq!(template.template_id, Some("greeting".to_owned()));
    assert_eq!(template.model, Some("gpt-4".to_owned()));
    assert_eq!(
        template.input_schema.get("name").unwrap().as_str(),
        Some("string")
    );
    assert_eq!(template.output_format, Some("text".to_owned()));
    assert_eq!(
        template.output_schema.get("answer").unwrap().as_str(),
        Some("string")
    );
    assert!(!template.optional_params.contains_key("model"));
    assert!(!template.optional_params.contains_key("input"));
    assert!(!template.optional_params.contains_key("output"));
    assert_eq!(
        template
            .optional_params
            .get("temperature")
            .unwrap()
            .as_str(),
        None
    );
}

#[rstest]
fn empty_metadata_yields_empty_schemas_and_no_model() {
    let template = PromptTemplate::new("hi".to_owned(), Object::new(), None).unwrap();
    assert_eq!(template.model, None);
    assert!(template.input_schema.is_empty());
    assert_eq!(template.output_format, None);
    assert!(template.output_schema.is_empty());
    assert!(template.optional_params.is_empty());
}

#[rstest]
fn a_non_string_model_is_rejected() {
    let metadata = metadata(r#"{"model": 4}"#);
    PromptTemplate::new("hi".to_owned(), metadata, None).unwrap_err();
}

#[rstest]
fn a_non_mapping_input_section_is_rejected() {
    let metadata = metadata(r#"{"input": "nope"}"#);
    PromptTemplate::new("hi".to_owned(), metadata, None).unwrap_err();
}

#[rstest]
fn a_non_mapping_input_schema_is_rejected() {
    let metadata = metadata(r#"{"input": {"schema": "nope"}}"#);
    PromptTemplate::new("hi".to_owned(), metadata, None).unwrap_err();
}

#[rstest]
fn a_non_string_output_format_is_rejected() {
    let metadata = metadata(r#"{"output": {"format": 1}}"#);
    PromptTemplate::new("hi".to_owned(), metadata, None).unwrap_err();
}
