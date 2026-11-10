use crate::bedrock_tools::native::is_bedrock_tool_block;

use super::prelude::json;

#[rstest::rstest]
#[case::a_system_tool(r#"{"systemTool":{"name":"nova_grounding"}}"#, true)]
#[case::a_tool_spec(r#"{"toolSpec":{"name":"t"}}"#, true)]
#[case::a_cache_point(r#"{"cachePoint":{"type":"default"}}"#, true)]
#[case::a_null_native_value(r#"{"toolSpec":null}"#, true)]
#[case::a_native_block_beside_a_function(r#"{"function":{"name":"f"},"cachePoint":{}}"#, true)]
#[case::a_lookalike_key(r#"{"systemToolish":{}}"#, false)]
#[case::an_empty_object("{}", false)]
#[case::a_snake_case_cache_point(r#"{"cache_point":{"type":"default"}}"#, false)]
#[case::a_string(r#""junk""#, false)]
#[case::a_list("[]", false)]
#[case::null("null", false)]
fn bedrock_tools_behavior_tests_behavior(#[case] tool: &str, #[case] expected: bool) {
    assert_eq!(is_bedrock_tool_block(&json(tool)), expected);
}
