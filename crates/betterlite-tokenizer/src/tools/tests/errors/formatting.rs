use std::mem::discriminant;

use rstest::rstest;

use crate::tools::error::ToolFormatError;

use super::super::format;

#[rstest]
#[case::missing_array_items(
    r#"[{"type":"function","function":{"name":"broken","parameters":{"type":"object","properties":{"values":{"type":"array"}}}}}]"#,
    ToolFormatError::MissingItems { field: "items" }
)]
#[case::enum_with_an_object(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","enum":[{"red":1}]}}}}}]"#,
    ToolFormatError::NotScalar { field: "enum" }
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
