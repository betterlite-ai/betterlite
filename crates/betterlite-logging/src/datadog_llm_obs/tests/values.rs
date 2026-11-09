use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::json;
use crate::datadog_llm_obs::values::{
    as_object, as_text, content_blocks, is_filled, mapping_field, safe_identifier, text_field,
};

fn object_fields(text: &str) -> indexmap::IndexMap<String, Json> {
    let value = json(text);
    let Json::Object(fields) = value else {
        panic!("the fixture is an object");
    };
    fields
}

#[rstest]
#[case::string(r#""python""#, "python")]
#[case::empty_string(r#""""#, "")]
#[case::number("7", "")]
#[case::boolean("true", "")]
#[case::null("null", "")]
#[case::list("[1]", "")]
#[case::object(r#"{"a": 1}"#, "")]
fn safe_identifier_keeps_only_strings(#[case] source: &str, #[case] expected: &str) {
    let value = json(source);
    assert_eq!(safe_identifier(Some(&value)), expected);
}

#[test]
fn safe_identifier_reads_an_absent_field_as_empty() {
    assert_eq!(safe_identifier(None), String::new());
}

#[rstest]
#[case::string(r#"{"name": "value"}"#, "value")]
#[case::number(r#"{"name": 7}"#, "")]
#[case::null(r#"{"name": null}"#, "fallback")]
#[case::absent(r#"{"other": 1}"#, "fallback")]
fn text_field_reads_strings_with_a_default_for_absent_and_null(
    #[case] source: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        text_field(&object_fields(source), "name", "fallback"),
        expected
    );
}

#[test]
fn mapping_field_returns_only_objects() {
    let fields =
        object_fields(r#"{"object": {"a": 1}, "list": [1], "text": "x", "nothing": null}"#);
    assert!(mapping_field(&fields, "object").is_some());
    assert!(mapping_field(&fields, "list").is_none());
    assert!(mapping_field(&fields, "text").is_none());
    assert!(mapping_field(&fields, "nothing").is_none());
    assert!(mapping_field(&fields, "absent").is_none());
}

#[test]
fn content_blocks_keeps_the_objects_of_a_content_list() {
    let fields =
        object_fields(r#"{"content": [{"type": "text", "text": "a"}, 7, {"type": "image"}]}"#);
    let blocks = content_blocks(&fields);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].get("type").and_then(as_text), Some("text"));
    assert_eq!(blocks[1].get("type").and_then(as_text), Some("image"));
}

#[rstest]
#[case::plain_string(r#""text""#)]
#[case::no_content(r#"{"role": "user"}"#)]
#[case::text_content(r#"{"content": "text"}"#)]
fn content_blocks_reads_a_non_list_content_as_no_blocks(#[case] source: &str) {
    let value = json(source);
    let empty = indexmap::IndexMap::new();
    let object = as_object(&value).unwrap_or(&empty);
    assert!(content_blocks(object).is_empty());
}

#[rstest]
#[case::object(r#"{"a": 1}"#, true)]
#[case::list("[1]", true)]
#[case::text(r#""x""#, true)]
#[case::number("7", true)]
#[case::zero("0", false)]
#[case::true_value("true", true)]
#[case::false_value("false", false)]
#[case::null("null", false)]
#[case::empty_text(r#""""#, false)]
#[case::empty_list("[]", false)]
#[case::empty_object("{}", false)]
fn is_filled_matches_python_truthiness(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(is_filled(&json(source)), expected);
}

#[test]
fn json_accessors_read_their_own_variant() {
    let object = json(r#"{"a": 1}"#);
    assert!(as_object(&object).is_some());
    assert!(as_text(&object).is_none());
    let text = json(r#""x""#);
    assert_eq!(as_text(&text), Some("x"));
}
