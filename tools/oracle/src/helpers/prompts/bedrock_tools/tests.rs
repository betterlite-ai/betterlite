#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::valid_names_round_trip(
    r#"{"call":"valid_names","names":["my tool!","already_valid"]}"#,
    json!([
        {"input": "my tool!", "valid": "my_tool_", "reverse": "my tool!"},
        {"input": "already_valid", "valid": "already_valid", "reverse": "already_valid"}
    ])
)]
#[case::is_native_classifies_a_block(
    r#"{"call":"is_native","tools":[{"toolSpec":null},"junk"]}"#,
    json!([true, false])
)]
#[case::tools_reports_blocks_and_reverse_names(
    r#"{"call":"tools","model":null,"capability":{"cache_points":"unsupported","supports_strict_tools":false},"tools":[{"type":"function","function":{"name":"my tool"}}]}"#,
    json!({
        "tools": [{"toolSpec": {
            "inputSchema": {"json": {"type": "object", "properties": {}, "required": []}},
            "name": "my_tool",
            "description": "my_tool"
        }}],
        "reverse": {"my_tool": "my tool"}
    })
)]
#[case::cache_point_returns_null_without_a_control(
    r#"{"call":"cache_point","scenarios":[{"model":null,"tool":{},"capability":{"cache_points":"short_ttl","supports_strict_tools":false}}]}"#,
    json!([null])
)]
fn bedrock_tools_oracle_adapter_tests(#[case] request: &str, #[case] expected: Value) {
    let input: Input = serde_json::from_str(request).expect("request is valid JSON");
    let value = Op::run(&input).expect("case is expected to succeed");
    assert_eq!(value, expected);
}
