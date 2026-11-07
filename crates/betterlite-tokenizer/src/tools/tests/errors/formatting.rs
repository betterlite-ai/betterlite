use std::mem::discriminant;

use rstest::rstest;

use crate::tools::error::ToolFormatError;

use super::super::format;

#[rstest]
#[case::missing_array_items(
    r#"[{"type":"function","function":{"name":"broken","parameters":{"type":"object","properties":{"values":{"type":"array"}}}}}]"#,
    ToolFormatError::MissingItems { field: "items" }
)]
#[case::enum_without_an_array(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","enum":"red"}}}}}]"#,
    ToolFormatError::NotArray { field: "enum" }
)]
#[case::enum_with_an_object(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","enum":[{"red":1}]}}}}}]"#,
    ToolFormatError::NotScalar { field: "enum" }
)]
#[case::non_text_required(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string"}},"required":"field"}}}]"#,
    ToolFormatError::NotTextArray { field: "required" }
)]
#[case::non_object_items(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"array","items":5}}}}}]"#,
    ToolFormatError::NotAnObject { field: "items" }
)]
#[case::non_object_properties(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":[]}}}]"#,
    ToolFormatError::NotAnObject { field: "properties" }
)]
#[case::non_text_property_description(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","description":7}}}}}]"#,
    ToolFormatError::NotText { field: "description" }
)]
fn token_formatting_behavior_tests_behavior(
    #[case] tools: &str,
    #[case] expected: ToolFormatError,
) {
    let Err(error) = format(tools) else {
        panic!("the formatter accepted invalid tools");
    };
    assert_eq!(discriminant(&error), discriminant(&expected));
    assert_eq!(error.to_string(), expected.to_string());
}
