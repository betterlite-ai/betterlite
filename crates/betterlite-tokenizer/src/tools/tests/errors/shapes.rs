use rstest::rstest;

use crate::tools::error::ToolFormatError;

use super::super::format;

#[rstest]
#[case::non_object_function(r#"[{"type":"function","function":"get_weather"}]"#, 0, "function")]
#[case::non_object_parameters(
    r#"[{"type":"function","function":{"name":"bad","parameters":[]}}]"#,
    0,
    "parameters"
)]
#[case::non_object_properties(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":[]}}}]"#,
    0,
    "properties"
)]
#[case::non_text_description(
    r#"[{"type":"function","function":{"name":"bad","description":7}}]"#,
    0,
    "description"
)]
#[case::non_text_property_description(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","description":7}}}}}]"#,
    0,
    "description"
)]
#[case::enum_without_an_array(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","enum":"red"}}}}}]"#,
    0,
    "enum"
)]
#[case::enum_with_an_object(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string","enum":[{"red":1}]}}}}}]"#,
    0,
    "enum"
)]
#[case::non_text_required(
    r#"[{"type":"function","function":{"name":"bad","parameters":{"properties":{"field":{"type":"string"}},"required":"field"}}}]"#,
    0,
    "required"
)]
#[case::non_array_declarations(r#"[{"functionDeclarations":"nope"}]"#, 0, "Declarations")]
#[case::non_object_declaration(r#"[{"functionDeclarations":["nope"]}]"#, 0, "Declarations")]
#[case::second_tool_shape_error(
    r#"[{"type":"function","function":{"name":"ok"}},{"type":"function","function":"bad"}]"#,
    1,
    "function"
)]
fn token_formatting_behavior_tests_behavior(
    #[case] tools: &str,
    #[case] index: usize,
    #[case] field: &str,
) {
    let Err(ToolFormatError::Shape {
        index: actual,
        source,
    }) = format(tools)
    else {
        panic!("the formatter accepted an invalid tool definition");
    };
    assert_eq!(actual, index);
    assert!(source.path().to_string().contains(field));
}
