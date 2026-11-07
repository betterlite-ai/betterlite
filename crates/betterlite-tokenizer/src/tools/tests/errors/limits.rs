use rstest::rstest;

use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

#[rstest]
#[case::at_the_byte_limit(b"[]", ToolLimits { max_bytes: 2, max_tools: 8 }, None)]
#[case::at_the_tool_limit(b"[{},{}]", ToolLimits { max_bytes: 64, max_tools: 2 }, None)]
#[case::over_the_byte_limit(
    b"[]",
    ToolLimits { max_bytes: 1, max_tools: 8 },
    Some("the tool definitions are 2 bytes, over the 1 byte limit")
)]
#[case::over_the_tool_limit(
    b"[{},{},{}]",
    ToolLimits { max_bytes: 64, max_tools: 1 },
    Some("the tool definitions hold 3 entries, over the 1 entry limit")
)]
#[case::over_the_function_limit(
    br#"[{"functionDeclarations":[{"name":"a"},{"name":"b"},{"name":"c"}]}]"#,
    ToolLimits { max_bytes: 1024, max_tools: 2 },
    Some("the tool definitions hold 3 entries, over the 2 entry limit")
)]
fn token_formatting_behavior_tests_behavior(
    #[case] bytes: &[u8],
    #[case] limits: ToolLimits,
    #[case] message: Option<&str>,
) {
    let result = parse_tool_definitions(bytes, limits)
        .and_then(|definitions| format_function_definitions(&definitions));
    match (result, message) {
        (Ok(_), None) => {}
        (Err(error), Some(expected)) => assert_eq!(error.to_string(), expected),
        (Ok(_), Some(expected)) => panic!("the parser accepted input it should reject: {expected}"),
        (Err(error), None) => panic!("the parser rejected input at its limit: {error}"),
    }
}
