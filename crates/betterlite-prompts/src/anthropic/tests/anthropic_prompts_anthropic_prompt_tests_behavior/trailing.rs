use rstest::rstest;

use crate::anthropic::assembly::{AssemblyError, strip_trailing_whitespace};
use crate::anthropic::error::AnthropicPromptError;
use crate::arguments::json::Json;

use super::fixtures::{converted, expected, failure};

fn stripped(raw: &str) -> Vec<Json> {
    let mut messages: Vec<Json> = serde_json::from_str(raw).unwrap();
    strip_trailing_whitespace(&mut messages).expect("the strip must succeed");
    messages
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_the_final_assistant_text() {
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"answer  \n"}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"text","text":"answer"}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_keeps_user_trailing_whitespace() {
    let input = r#"[{"role":"user","content":"hi  "}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"hi  "}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_only_text_blocks_of_the_last_turn()
 {
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"answer  ","thinking_blocks":[{"type":"thinking","thinking":"t  ","signature":"s"}]}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"thinking","thinking":"t  ","signature":"s"},{"type":"text","text":"answer"}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_a_string_content_assistant_turn() {
    assert_eq!(
        stripped(r#"[{"role":"assistant","content":"answer  "}]"#),
        expected(r#"[{"role":"assistant","content":"answer"}]"#)
    );
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_a_trailing_unit_separator() {
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"answer\u001f"}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"text","text":"answer"}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_unit_separators_only_at_the_end()
{
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"first\u001e"},{"role":"user","content":"again"},{"role":"assistant","content":"answer  \u001f"}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"text","text":"first\u001e"}]},{"role":"user","content":[{"type":"text","text":"again"}]},{"role":"assistant","content":[{"type":"text","text":"answer"}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_unit_separators_from_text_blocks() {
    assert_eq!(
        stripped(r#"[{"role":"assistant","content":[{"type":"text","text":"answer  \u001c"}]}]"#),
        expected(r#"[{"role":"assistant","content":[{"type":"text","text":"answer"}]}]"#)
    );
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_unit_separators_from_string_content() {
    assert_eq!(
        stripped(r#"[{"role":"assistant","content":"answer\u001d"}]"#),
        expected(r#"[{"role":"assistant","content":"answer"}]"#)
    );
}

#[rstest]
#[case::non_text_block_without_a_type(r#"[{"role":"assistant","content":[{"content":"x"}]}]"#)]
#[case::text_block_without_text(r#"[{"role":"assistant","content":[{"type":"text"}]}]"#)]
#[case::text_block_with_non_text_text(
    r#"[{"role":"assistant","content":[{"type":"text","text":5}]}]"#
)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_trailing_block_failures(#[case] raw: &str) {
    let mut messages: Vec<Json> = serde_json::from_str(raw).unwrap();
    let error = strip_trailing_whitespace(&mut messages).expect_err("the strip must fail");
    assert!(matches!(error, AssemblyError::TrailingBlock { .. }));
}

#[rstest]
#[case::empty_list("[]")]
#[case::last_message_is_not_an_object("[7]")]
#[case::assistant_without_content(r#"[{"role":"assistant"}]"#)]
#[case::assistant_with_null_content(r#"[{"role":"assistant","content":null}]"#)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_trailing_strip_leaves_these_alone(
    #[case] raw: &str,
) {
    assert_eq!(stripped(raw), expected(raw));
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_a_non_text_block_type_is_left_alone() {
    assert_eq!(
        stripped(r#"[{"role":"assistant","content":[{"type":5,"text":"x  "}]}]"#),
        expected(r#"[{"role":"assistant","content":[{"type":5,"text":"x  "}]}]"#)
    );
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_strips_text_after_a_non_object_compaction_block()
 {
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","provider_specific_fields":{"compaction_blocks":[5,{"type":"text","text":"answer  "},{"type":"compaction","content":"x"}]}}]"#;
    let output = r#"[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[5,{"type":"text","text":"answer"},{"type":"compaction","content":"x"}]}]"#;
    assert_eq!(converted(input, false).await, expected(output));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_a_compaction_block_without_a_type_fails()
{
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","provider_specific_fields":{"compaction_blocks":[{"content":"x"}]}}]"#;
    let error = failure(input, false).await;
    assert!(matches!(
        error,
        AnthropicPromptError::Assembly(AssemblyError::TrailingBlock {
            block: 0,
            field: "type"
        })
    ));
}

#[tokio::test]
async fn anthropic_prompts_anthropic_prompt_tests_behavior_a_compaction_text_block_without_text_fails()
 {
    let input = r#"[{"role":"user","content":"hi"},{"role":"assistant","provider_specific_fields":{"compaction_blocks":[{"type":"text","text":7}]}}]"#;
    let error = failure(input, false).await;
    assert!(matches!(
        error,
        AnthropicPromptError::Assembly(AssemblyError::TrailingBlock {
            block: 0,
            field: "text"
        })
    ));
}
