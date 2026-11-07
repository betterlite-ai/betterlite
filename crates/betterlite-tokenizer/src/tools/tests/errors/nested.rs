use rstest::rstest;

use crate::tools::error::ToolFormatError;

use super::super::format;

#[rstest]
#[case::enum_without_an_array(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","enum":"red"}}}}}]"#,
    "enum"
)]
#[case::non_text_required(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string"}},"required":"field"}}}]"#,
    "required"
)]
#[case::non_object_items(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"array","items":5}}}}}]"#,
    "items"
)]
#[case::non_object_properties(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":[]}}}]"#,
    "properties"
)]
#[case::non_text_property_description(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","description":7}}}}}]"#,
    "description"
)]
fn token_formatting_behavior_tests_behavior(#[case] tools: &str, #[case] field: &str) {
    let Err(error) = format(tools) else {
        panic!("the formatter accepted invalid tools");
    };
    assert!(error.to_string().contains(field));
    let ToolFormatError::Nested { field: actual, .. } = error else {
        panic!("the error is not a nested shape error");
    };
    assert_eq!(actual, field);
}
