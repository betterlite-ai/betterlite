use rstest::rstest;

use crate::anthropic::error::AnthropicPromptError;
use crate::anthropic_user_blocks::prelude::PrepareError;

use super::fixtures::{converted, expected, failure};

const ORPHAN_CALL: &str = r#"[{"role":"user","content":"u"},{"role":"tool","tool_call_id":"call_9","content":"stray"},{"role":"user","content":"next"}]"#;
const MISSING_RESULT_CALL: &str = r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"user","content":"next"}]"#;

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_inserts_a_named_missing_result() {
    let output = converted(MISSING_RESULT_CALL, true).await;
    assert_eq!(
        output,
        expected(
            r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"f","input":{}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"[System: Tool execution skipped/interrupted by user. No result provided for tool 'f'.]"},{"type":"text","text":"next"}]}]"#
        )
    );
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_leaves_a_missing_result_alone_without_modify_params()
 {
    let output = converted(MISSING_RESULT_CALL, false).await;
    assert_eq!(
        output,
        expected(
            r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"f","input":{}}]},{"role":"user","content":[{"type":"text","text":"next"}]}]"#
        )
    );
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_removes_an_orphaned_result() {
    let output = converted(ORPHAN_CALL, true).await;
    assert_eq!(
        output,
        expected(
            r#"[{"role":"user","content":[{"type":"text","text":"u"},{"type":"text","text":"next"}]}]"#
        )
    );
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_keeps_the_last_duplicate_result() {
    let input = r#"[{"role":"user","content":"u"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","content":"first"},{"role":"tool","tool_call_id":"call_1","content":"last"}]"#;
    let output = converted(input, true).await;
    assert_eq!(
        output,
        expected(
            r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"f","input":{}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"last"}]}]"#
        )
    );
}

#[rstest]
#[case::whitespace_user_content(
    r#"[{"role":"user","content":"   "}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"[System: Empty message content sanitised to satisfy protocol]"}]}]"#
)]
#[case::empty_assistant_text_block(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":[{"type":"text","text":""}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"[System: Empty message content sanitised to satisfy protocol]"}]}]"#
)]
#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_rewrites_empty_text(
    #[case] input: &str,
    #[case] output: &str,
) {
    assert_eq!(converted(input, false).await, expected(output));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_an_empty_request_needs_modify_params() {
    let error = failure("[]", false).await;
    assert!(matches!(
        error,
        AnthropicPromptError::Prepare(PrepareError::NoUserMessages)
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_an_empty_request_becomes_a_continue_turn()
 {
    assert_eq!(
        converted("[]", true).await,
        expected(r#"[{"role":"user","content":[{"type":"text","text":"Please continue."}]}]"#)
    );
}
