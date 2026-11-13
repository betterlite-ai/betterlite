use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use tokio::runtime::Builder;

use crate::bedrock::assembly::AssistantContinue;
use crate::bedrock::flags::{CachePointCapability, MessageCachePoints, PromptCapabilities};
use crate::bedrock::input::{MAX_PROMPT_INPUT_BYTES, PromptInputError, parse_prompt_bedrock_input};
use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};

use super::prelude::{Harness, Settings};

#[rstest]
#[case::valid_input(
    r#"{"messages":[{"role":"user","content":"hi"}],"model":"anthropic.claude-sonnet-4-5-20250929-v1:0"}"#,
    false
)]
#[case::modify_params_is_read(
    r#"{"messages":[],"modify_params":true,"assistant_continue_message":"go on"}"#,
    true
)]
fn bedrock_prompts_bedrock_prompt_tests_behavior_parses_the_fuzz_input(
    #[case] raw: &str,
    #[case] modify_params: bool,
) {
    let input = parse_prompt_bedrock_input(raw.as_bytes()).expect("the input parses");
    assert_eq!(input.modify_params, modify_params);
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_reads_the_continue_messages() {
    let input = parse_prompt_bedrock_input(
        br#"{"messages":[],"user_continue_message":{"role":"user","content":"carry on"},"assistant_continue_message":{"role":"assistant","content":"go on"}}"#,
    )
    .expect("the input parses");
    let user_continue = input
        .user_continue_message
        .expect("the user message parses");
    assert_eq!(user_continue.role, "user");
    assert!(matches!(
        input.assistant_continue_message,
        Some(AssistantContinue::Message(_))
    ));
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_reads_a_string_assistant_continue_message() {
    let input =
        parse_prompt_bedrock_input(br#"{"messages":[],"assistant_continue_message":"go on"}"#)
            .expect("the input parses");
    assert!(matches!(
        input.assistant_continue_message,
        Some(AssistantContinue::Text(text)) if text == "go on"
    ));
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_reads_the_injected_model_flags() {
    let input = parse_prompt_bedrock_input(
        br#"{"messages":[],"capabilities":{"cache_points":"extended_ttl","supports_strict_tools":true}}"#,
    )
    .expect("the input parses");
    assert_eq!(
        input.capabilities,
        PromptCapabilities {
            cache_points: CachePointCapability::ExtendedTtl,
            supports_strict_tools: true,
        }
    );
    assert_eq!(
        BedrockToolCapabilities::from(input.capabilities),
        BedrockToolCapabilities {
            cache_points: CachePointSupport::ExtendedTtl,
            supports_strict_tools: true,
        }
    );
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_rejects_input_over_the_byte_limit() {
    let bytes = vec![b' '; MAX_PROMPT_INPUT_BYTES.saturating_add(1)];
    let error = parse_prompt_bedrock_input(&bytes).expect_err("the input is refused");
    assert!(matches!(
        error,
        PromptInputError::TooLarge {
            length,
            limit: MAX_PROMPT_INPUT_BYTES
        } if length == MAX_PROMPT_INPUT_BYTES.saturating_add(1)
    ));
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_accepts_input_at_the_byte_limit() {
    let mut bytes = br#"{"messages":[]}"#.to_vec();
    bytes.resize(MAX_PROMPT_INPUT_BYTES, b' ');
    let input = parse_prompt_bedrock_input(&bytes).expect("an input at the limit parses");
    assert!(input.messages.is_empty());
}

#[rstest]
#[case::missing_messages("{}")]
#[case::messages_not_a_list(r#"{"messages":{}}"#)]
#[case::unknown_field(r#"{"messages":[],"drop_params":true}"#)]
fn bedrock_prompts_bedrock_prompt_tests_behavior_rejects_json_of_the_wrong_shape(
    #[case] raw: &str,
) {
    let error = parse_prompt_bedrock_input(raw.as_bytes()).expect_err("the input is refused");
    assert!(matches!(error, PromptInputError::Decode { .. }));
}

#[rstest]
fn replays_the_prompt_bedrock_corpus(#[files("fuzz/corpus/prompt_bedrock/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(input) = parse_prompt_bedrock_input(&bytes) else {
        return;
    };
    let settings = Settings {
        model: input.model.as_deref(),
        capabilities: BedrockToolCapabilities::from(input.capabilities),
        cache_points: MessageCachePoints {
            support: CachePointSupport::from(input.capabilities.cache_points),
        },
        user_continue_message: input.user_continue_message.as_ref(),
        assistant_continue_message: input.assistant_continue_message.as_ref(),
        modify_params: input.modify_params,
    };
    let harness = Harness::new();
    let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
        return;
    };
    drop(runtime.block_on(harness.convert(&input.messages, &input.tools, &settings)));
}
