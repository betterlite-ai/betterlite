use crate::tools::error::ToolFormatError;

use super::super::format;

#[test]
fn token_formatting_behavior_tests_behavior() {
    let tools = r#"[{"type":"function","function":{"name":"broken","parameters":{"type":"object","properties":{"values":{"type":"array"}}}}}]"#;
    let Err(error) = format(tools) else {
        panic!("the formatter accepted an array schema without items");
    };
    assert!(matches!(
        error,
        ToolFormatError::MissingItems { field: "items" }
    ));
}
