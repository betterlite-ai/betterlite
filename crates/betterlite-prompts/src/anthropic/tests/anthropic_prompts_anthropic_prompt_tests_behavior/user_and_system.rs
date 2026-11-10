use rstest::rstest;

use crate::anthropic::assembly::AssemblyError;
use crate::anthropic::error::AnthropicPromptError;
use crate::arguments::json::Json;

use super::fixtures::{MODEL, convert_values, converted, expected, failure, messages};

#[rstest]
#[case::text_user_turn(
    r#"[{"role":"user","content":"hi"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#
)]
#[case::consecutive_user_type_messages_merge(
    r#"[{"role":"user","content":"a"},{"role":"tool","tool_call_id":"call_1","content":"r"},{"role":"user","content":"b"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"tool_result","tool_use_id":"call_1","content":"r"},{"type":"text","text":"b"}]}]"#
)]
#[case::null_content_is_omitted(
    r#"[{"role":"user","content":null},{"role":"user","content":"b"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"b"}]}]"#
)]
#[case::empty_content_list_is_omitted(
    r#"[{"role":"user","content":[]},{"role":"user","content":"b"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"b"}]}]"#
)]
#[case::assistant_turn_stays_distinct_from_the_user_turn(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]}]"#
)]
#[case::consecutive_assistant_messages_merge_into_one_turn(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"assistant","content":"b"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]"#
)]
#[case::a_system_run_becomes_wire_system_messages(
    r#"[{"role":"system","content":"lead"},{"role":"user","content":"u"}]"#,
    r#"[{"role":"system","content":[{"type":"text","text":"lead"}]},{"role":"user","content":[{"type":"text","text":"u"}]}]"#
)]
#[case::system_list_content_keeps_only_text_and_block_cache_control(
    r#"[{"role":"system","content":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","ttl":"1h"}},{"type":"image_url","image_url":{"url":"x"}},{"type":"text","text":""}]},{"role":"user","content":"u"}]"#,
    r#"[{"role":"system","content":[{"type":"text","text":"s","cache_control":{"type":"ephemeral","ttl":"1h"}}]},{"role":"user","content":[{"type":"text","text":"u"}]}]"#
)]
#[case::a_system_message_without_text_is_omitted(
    r#"[{"role":"system","content":""},{"role":"user","content":"u"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]}]"#
)]
#[case::a_mid_conversation_system_run_stays_in_place(
    r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"system","content":"reminder"},{"role":"system","content":[{"type":"text","text":"second","cache_control":{"type":"ephemeral","ttl":"5m"}}]},{"role":"user","content":"u2"}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]},{"role":"system","content":[{"type":"text","text":"reminder"}]},{"role":"system","content":[{"type":"text","text":"second","cache_control":{"type":"ephemeral","ttl":"5m"}}]},{"role":"user","content":[{"type":"text","text":"u2"}]}]"#
)]
#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior(
    #[case] input: &str,
    #[case] output: &str,
) {
    assert_eq!(converted(input, false).await, expected(output));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_unsupported_role_names_the_message() {
    let error = failure(
        r#"[{"role":"user","content":"u"},{"role":"developer","content":"x"}]"#,
        false,
    )
    .await;
    assert!(matches!(
        error,
        AnthropicPromptError::Assembly(AssemblyError::UnsupportedRole { index: 1, role })
            if role == "developer"
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_missing_role_names_the_message() {
    let error = failure(
        r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"content":"x"}]"#,
        false,
    )
    .await;
    assert!(matches!(
        error,
        AnthropicPromptError::Assembly(AssemblyError::MissingRole { index: 2 })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_non_text_role_names_the_message() {
    let error = failure(
        r#"[{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":5}]"#,
        false,
    )
    .await;
    assert!(matches!(
        error,
        AnthropicPromptError::Assembly(AssemblyError::RoleNotText { index: 2 })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_system_content_of_the_wrong_type_is_refused()
 {
    let error = failure(
        r#"[{"role":"system","content":5},{"role":"user","content":"u"}]"#,
        false,
    )
    .await;
    assert!(matches!(
        error,
        AnthropicPromptError::Assembly(AssemblyError::SystemMessage { index: 0, .. })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_keeps_the_source_messages_unchanged() {
    let source = r#"[{"role":"user","content":[{"type":"text","text":"u"}]},{"role":"assistant","content":null,"thinking_blocks":[{"type":"thinking","thinking":"t","signature":"s"}],"tool_calls":[{"id":"call.1","type":"function","function":{"name":"f","arguments":"{}"}}],"cache_control":{"type":"ephemeral"}}]"#;
    let input = messages(source);
    let original = input.clone();
    let converted = convert_values(&input, MODEL, true, &[])
        .await
        .expect("converts");
    assert!(!converted.is_empty());
    assert_eq!(input, original);
    assert_eq!(Json::List(input), Json::List(messages(source)));
}
