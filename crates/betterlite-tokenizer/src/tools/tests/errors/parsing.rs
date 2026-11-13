use rstest::rstest;

use crate::tools::error::ToolFormatError;
use crate::tools::{ToolLimits, parse_tool_definitions};

#[rstest]
#[case::malformed_json(b"{not json")]
#[case::non_array_json(b"{}")]
#[case::truncated_array(b"[{\"name\":\"x\"")]
fn token_formatting_behavior_tests_behavior(#[case] bytes: &[u8]) {
    let Err(error) = parse_tool_definitions(bytes, ToolLimits::default()) else {
        panic!("the parser accepted invalid tool definitions");
    };
    assert!(matches!(error, ToolFormatError::Json { .. }));
}
