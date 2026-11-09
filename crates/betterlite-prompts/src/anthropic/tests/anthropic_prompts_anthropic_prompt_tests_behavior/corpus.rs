use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use tokio::runtime::Builder;

use crate::anthropic::assembly::{PromptDeps, anthropic_messages_pt};
use crate::anthropic::input::{
    MAX_PROMPT_INPUT_BYTES, PromptInputError, parse_prompt_anthropic_input,
};

use super::fixtures::{GENERATED_ID, NoFileBlocks, UrlMap};

#[rstest]
#[case::valid_input(
    r#"{"messages":[{"role":"user","content":"hi"}],"model":"claude-sonnet-4-5","llm_provider":"anthropic"}"#,
    false
)]
#[case::modify_params_is_read(
    r#"{"messages":[],"model":"m","llm_provider":"p","modify_params":true}"#,
    true
)]
fn anthropic_prompts_anthropic_prompt_tests_behavior_parses_the_fuzz_input(
    #[case] raw: &str,
    #[case] modify_params: bool,
) {
    let input = parse_prompt_anthropic_input(raw.as_bytes()).expect("the input parses");
    assert_eq!(input.modify_params, modify_params);
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_rejects_input_over_the_byte_limit() {
    let bytes = vec![b' '; MAX_PROMPT_INPUT_BYTES.saturating_add(1)];
    let error = parse_prompt_anthropic_input(&bytes).expect_err("the input is refused");
    assert!(matches!(
        error,
        PromptInputError::TooLarge {
            length,
            limit: MAX_PROMPT_INPUT_BYTES
        } if length == MAX_PROMPT_INPUT_BYTES.saturating_add(1)
    ));
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_accepts_input_at_the_byte_limit() {
    let mut bytes = br#"{"messages":[],"model":"m","llm_provider":"p"}"#.to_vec();
    bytes.resize(MAX_PROMPT_INPUT_BYTES, b' ');
    let input = parse_prompt_anthropic_input(&bytes).expect("an input at the limit parses");
    assert!(input.messages.is_empty());
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_rejects_json_of_the_wrong_shape() {
    let error =
        parse_prompt_anthropic_input(b"{\"messages\":[]}").expect_err("the input is refused");
    assert!(matches!(error, PromptInputError::Decode { .. }));
}

#[test]
fn anthropic_prompts_anthropic_prompt_tests_behavior_rejects_deeply_nested_input() {
    let mut raw = String::from(
        r#"{"messages":[{"role":"user","content":"hi","provider_specific_fields":{"deep":"#,
    );
    raw.push_str(&"[".repeat(200));
    raw.push_str(&"]".repeat(200));
    raw.push_str("}}],\"model\":\"m\",\"llm_provider\":\"p\"}");
    let error = parse_prompt_anthropic_input(raw.as_bytes()).expect_err("deep input is refused");
    assert!(matches!(error, PromptInputError::Decode { .. }));
}

#[rstest]
fn replays_the_prompt_anthropic_corpus(#[files("fuzz/corpus/prompt_anthropic/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(input) = parse_prompt_anthropic_input(&bytes) else {
        return;
    };
    let Ok(runtime) = Builder::new_current_thread().build() else {
        return;
    };
    let fetcher = UrlMap {
        data_urls: BTreeMap::new(),
    };
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let deps = PromptDeps {
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    drop(runtime.block_on(anthropic_messages_pt(
        &input.messages,
        &input.model,
        &input.llm_provider,
        input.modify_params,
        &deps,
    )));
}
