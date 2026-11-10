use rstest::rstest;

use crate::anthropic::assistant::{AssistantError, AssistantRun};
use crate::arguments::json::Json;

use super::fixtures::{GENERATED_ID, converted, expected, object};

fn assistant_content(message: &str) -> Vec<Json> {
    let mut run = AssistantRun::default();
    let new_id = || GENERATED_ID.to_owned();
    run.append(&object(message), 0, &new_id)
        .expect("the message must append");
    let Some(Json::Object(fields)) = run.into_message() else {
        return Vec::new();
    };
    match fields.get("content") {
        Some(Json::List(blocks)) => blocks.clone(),
        _ => Vec::new(),
    }
}

fn append_error(message: &str) -> AssistantError {
    let mut run = AssistantRun::default();
    let new_id = || GENERATED_ID.to_owned();
    run.append(&object(message), 0, &new_id)
        .expect_err("the message must be refused")
}

#[rstest]
#[case::content_string(
    r#"{"role":"assistant","content":"hello","cache_control":{"type":"ephemeral","ttl":"1h"}}"#,
    r#"[{"type":"text","text":"hello","cache_control":{"type":"ephemeral","ttl":"1h"}}]"#
)]
#[case::empty_content_string_adds_nothing(r#"{"role":"assistant","content":""}"#, "[]")]
#[case::missing_content_adds_nothing(r#"{"role":"assistant"}"#, "[]")]
#[case::text_block_uses_its_own_cache_control(
    r#"{"role":"assistant","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}],"cache_control":{"type":"ephemeral","ttl":"1h"}}"#,
    r#"[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]"#
)]
#[case::empty_text_block_adds_nothing(
    r#"{"role":"assistant","content":[{"type":"text","text":""}]}"#,
    "[]"
)]
#[case::text_block_without_text_adds_nothing(
    r#"{"role":"assistant","content":[{"type":"text"}]}"#,
    "[]"
)]
#[case::unknown_content_block_types_are_dropped(
    r#"{"role":"assistant","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}"#,
    "[]"
)]
#[case::a_content_block_without_a_type_is_dropped(
    r#"{"role":"assistant","content":[{"text":"orphan"}]}"#,
    "[]"
)]
#[case::server_tool_blocks_pass_through(
    r#"{"role":"assistant","content":[{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{}},{"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[]}]}"#,
    r#"[{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{}},{"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[]}]"#
)]
#[case::thinking_blocks_precede_the_text(
    r#"{"role":"assistant","content":"answer","thinking_blocks":[{"type":"thinking","thinking":"hmm","signature":"sig-1"},{"type":"redacted_thinking","data":"abc"}]}"#,
    r#"[{"type":"thinking","thinking":"hmm","signature":"sig-1"},{"type":"redacted_thinking","data":"abc"},{"type":"text","text":"answer"}]"#
)]
#[case::unsignable_thinking_blocks_are_dropped(
    r#"{"role":"assistant","content":"answer","thinking_blocks":[{"type":"thinking","thinking":"unsigned","signature":""},{"type":"thinking","thinking":"","signature":"sig"},{"type":"thinking","thinking":"foreign","signature":"litellm_encrypted_reasoning:x"}]}"#,
    r#"[{"type":"text","text":"answer"}]"#
)]
#[case::inline_thinking_suppresses_the_separate_blocks(
    r#"{"role":"assistant","content":[{"type":"thinking","thinking":"inline","signature":"sig-inline"},{"type":"text","text":"answer"}],"thinking_blocks":[{"type":"thinking","thinking":"separate","signature":"sig"}]}"#,
    r#"[{"type":"thinking","thinking":"inline","signature":"sig-inline"},{"type":"text","text":"answer"}]"#
)]
#[case::inline_thinking_with_empty_text_is_dropped(
    r#"{"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"sig-inline"},{"type":"text","text":"answer"}]}"#,
    r#"[{"type":"text","text":"answer"}]"#
)]
#[case::inline_unsigned_thinking_is_dropped(
    r#"{"role":"assistant","content":[{"type":"thinking","thinking":"hmm","signature":""},{"type":"text","text":"answer"}]}"#,
    r#"[{"type":"text","text":"answer"}]"#
)]
#[case::inline_redacted_thinking_is_kept_with_the_tool_call(
    r#"{"role":"assistant","content":[{"type":"redacted_thinking","data":"abc"},{"type":"text","text":"a"}],"tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}"#,
    r#"[{"type":"redacted_thinking","data":"abc"},{"type":"text","text":"a"},{"type":"tool_use","id":"call_1","name":"f","input":{}}]"#
)]
#[case::inline_redacted_thinking_suppresses_the_separate_blocks(
    r#"{"role":"assistant","content":[{"type":"redacted_thinking","data":"abc"},{"type":"text","text":"answer"}],"thinking_blocks":[{"type":"thinking","thinking":"separate","signature":"sig"}]}"#,
    r#"[{"type":"redacted_thinking","data":"abc"},{"type":"text","text":"answer"}]"#
)]
#[case::compaction_blocks_come_first(
    r#"{"role":"assistant","content":"answer","provider_specific_fields":{"compaction_blocks":[{"type":"compaction","content":"summary","signature":"sig"}]}}"#,
    r#"[{"type":"compaction","content":"summary","signature":"sig"},{"type":"text","text":"answer"}]"#
)]
#[case::a_non_list_compaction_field_is_ignored(
    r#"{"role":"assistant","content":"answer","provider_specific_fields":{"compaction_blocks":{"type":"compaction"}}}"#,
    r#"[{"type":"text","text":"answer"}]"#
)]
#[case::an_empty_compaction_list_adds_nothing(
    r#"{"role":"assistant","content":"answer","provider_specific_fields":{"compaction_blocks":[]}}"#,
    r#"[{"type":"text","text":"answer"}]"#
)]
fn anthropic_prompts_anthropic_prompt_tests_behavior(#[case] message: &str, #[case] blocks: &str) {
    assert_eq!(assistant_content(message), expected(blocks));
}

#[rstest]
#[case::non_object_block(
    r#"{"role":"assistant","content":[5]}"#,
    AssistantError::BlockNotObject {
        index: 0,
        block: 0,
    }
)]
#[case::non_text_block_type(
    r#"{"role":"assistant","content":[{"type":5}]}"#,
    AssistantError::BlockField {
        index: 0,
        block: 0,
        field: "type",
    }
)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_content_block_failures(
    #[case] message: &str,
    #[case] failure: AssistantError,
) {
    assert_eq!(append_error(message).to_string(), failure.to_string());
}

#[rstest]
#[case::non_string_thinking_text(
    r#"{"role":"assistant","content":[{"type":"thinking","thinking":5,"signature":"sig"}]}"#,
    AssistantError::BlockField {
        index: 0,
        block: 0,
        field: "thinking",
    }
)]
#[case::non_string_text(
    r#"{"role":"assistant","content":[{"type":"text","text":5}]}"#,
    AssistantError::BlockField {
        index: 0,
        block: 0,
        field: "text",
    }
)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_block_field_failures(
    #[case] message: &str,
    #[case] failure: AssistantError,
) {
    assert_eq!(append_error(message).to_string(), failure.to_string());
}

#[rstest]
#[case::thinking_blocks_not_a_list(
    r#"{"role":"assistant","content":"a","thinking_blocks":"sig"}"#,
    AssistantError::Field {
        index: 0,
        field: "thinking_blocks",
    }
)]
#[case::a_thinking_block_is_not_an_object(
    r#"{"role":"assistant","content":"a","thinking_blocks":[7]}"#,
    AssistantError::BlockNotObject {
        index: 0,
        block: 0,
    }
)]
#[case::provider_fields_not_an_object(
    r#"{"role":"assistant","content":"a","provider_specific_fields":[]}"#,
    AssistantError::Field {
        index: 0,
        field: "provider_specific_fields",
    }
)]
#[case::content_of_the_wrong_type(
    r#"{"role":"assistant","content":5}"#,
    AssistantError::Field {
        index: 0,
        field: "content",
    }
)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_message_failures(
    #[case] message: &str,
    #[case] failure: AssistantError,
) {
    assert_eq!(append_error(message).to_string(), failure.to_string());
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_thinking_blocks_through_the_request() {
    let input = r#"[{"role":"user","content":"think"},{"role":"assistant","content":"answer","thinking_blocks":[{"type":"thinking","thinking":"hmm","signature":"sig-1"},{"type":"thinking","thinking":"unsigned","signature":""},{"type":"thinking","thinking":"","signature":"sig-2"},{"type":"redacted_thinking","data":"abc"},{"type":"thinking","thinking":"foreign","signature":"litellm_encrypted_reasoning:xyz"}]}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"think"}]},{"role":"assistant","content":[{"type":"thinking","thinking":"hmm","signature":"sig-1"},{"type":"redacted_thinking","data":"abc"},{"type":"text","text":"answer"}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}
