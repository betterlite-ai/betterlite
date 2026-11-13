use crate::arguments::json::Json;
use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use crate::bedrock_tools::error::BedrockToolError;
use crate::bedrock_tools::parse::tools_from_json_bytes;

use super::prelude::json;

enum Expected {
    Blocks(&'static str),
    Json,
    NotObject(usize),
    WrongType(&'static str),
}

const CAPABILITIES: BedrockToolCapabilities = BedrockToolCapabilities {
    cache_points: CachePointSupport::Unsupported,
    supports_strict_tools: false,
};

#[rstest::rstest]
#[case::rejects_empty_bytes(b"", Expected::Json)]
#[case::rejects_malformed_json(br#"{"tools":[{"type":"function"}"#, Expected::Json)]
#[case::rejects_an_object_instead_of_an_array(br#"{"type":"function"}"#, Expected::Json)]
#[case::accepts_an_empty_array(b"[]", Expected::Blocks("[]"))]
#[case::converts_a_tool_from_bytes(
    br#"[{"type":"function","function":{"name":"t"}}]"#,
    Expected::Blocks(r#"[{"toolSpec":{"inputSchema":{"json":{"type":"object","properties":{},"required":[]}},"name":"t","description":"t"}}]"#),
)]
#[case::reports_a_non_object_tool(br#"["junk"]"#, Expected::NotObject(0))]
#[case::reports_a_bad_tool_field(
    br#"[{"type":"function","function":{"name":7}}]"#,
    Expected::WrongType("name")
)]
fn bedrock_tools_behavior_tests_behavior(#[case] bytes: &[u8], #[case] expected: Expected) {
    let result = tools_from_json_bytes(bytes, CAPABILITIES);
    match expected {
        Expected::Blocks(raw) => assert_eq!(Json::List(result.unwrap().blocks), json(raw)),
        Expected::Json => assert!(matches!(result, Err(BedrockToolError::Json { .. }))),
        Expected::NotObject(index) => assert!(matches!(
            result,
            Err(BedrockToolError::ToolNotObject { index: actual }) if actual == index
        )),
        Expected::WrongType(field) => assert!(matches!(
            result,
            Err(BedrockToolError::WrongType { field: actual }) if actual == field
        )),
    }
}
