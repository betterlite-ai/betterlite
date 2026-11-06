use super::*;
use crate::response_tool_calls::normalized::anthropic_messages_tool_calls;
use rstest::rstest;

#[rstest]
#[case::tool_use_blocks_only(
    r#"{"content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"toolu_1","name":"search","input":{"query":"x"}}]}"#,
    r#"[{"id":"toolu_1","name":"search","arguments":{"query":"x"}}]"#
)]
#[case::non_object_input_gives_empty_arguments(
    r#"{"content":[{"type":"tool_use","id":"t","name":"n","input":"{\"q\":1}"}]}"#,
    r#"[{"id":"t","name":"n","arguments":{}}]"#
)]
#[case::missing_input_gives_empty_arguments(
    r#"{"content":[{"type":"tool_use","id":"t","name":"n"}]}"#,
    r#"[{"id":"t","name":"n","arguments":{}}]"#
)]
#[case::block_id_and_name_are_taken_as_sent(
    r#"{"content":[{"type":"tool_use","id":"","name":"","input":{}}]}"#,
    r#"[{"id":"","name":"","arguments":{}}]"#
)]
#[case::block_without_id_or_name_yields_nulls(
    r#"{"content":[{"type":"tool_use","input":{}}]}"#,
    r#"[{"id":null,"name":null,"arguments":{}}]"#
)]
#[case::absent_content_yields_nothing("{}", "[]")]
#[case::every_tool_use_block_is_kept(
    r#"{"content":[{"type":"tool_use","id":"a","name":"one","input":{"n":1}},{"type":"tool_use","id":"b","name":"two","input":{"n":2}}]}"#,
    r#"[{"id":"a","name":"one","arguments":{"n":1}},{"id":"b","name":"two","arguments":{"n":2}}]"#
)]
fn response_tool_calls_behavior_tests_behavior(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(
        anthropic_messages_tool_calls(&response(json)),
        calls(expected)
    );
}
