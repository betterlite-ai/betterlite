use rstest::rstest;

use crate::tools::error::ToolFormatError;

use super::super::format;

#[rstest]
#[case::function_name_not_text(r#"[{"function":{"name":7}}]"#, 0, "function", "name")]
#[case::function_description_not_text(
    r#"[{"function":{"name":"bad","description":7}}]"#,
    0,
    "function",
    "description"
)]
#[case::declaration_name_not_text(
    r#"[{"functionDeclarations":[{"name":7}]}]"#,
    0,
    "functionDeclarations",
    "name"
)]
#[case::inline_name_not_text(r#"[{"name":7}]"#, 0, "name", "")]
#[case::inline_description_not_text(r#"[{"description":7,"name":"x"}]"#, 0, "description", "")]
#[case::second_tool_shape_error(
    r#"[{"type":"function","function":{"name":"ok"}},{"type":"function","function":{"name":7}}]"#,
    1,
    "function",
    "name"
)]
fn token_formatting_behavior_tests_behavior(
    #[case] tools: &str,
    #[case] index: usize,
    #[case] field: &str,
    #[case] path: &str,
) {
    let Err(ToolFormatError::Shape {
        index: actual,
        field: actual_field,
        source,
    }) = format(tools)
    else {
        panic!("the formatter accepted an invalid tool definition");
    };
    assert_eq!(actual, index);
    assert_eq!(actual_field, field);
    assert!(source.path().to_string().contains(path));
}
