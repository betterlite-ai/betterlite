#![cfg(test)]

use std::sync::Mutex;

use super::fallback;
use crate::builtin::Message;
use crate::factory::Output;
use crate::factory::PromptFactory;
use crate::factory::hub::{Hub, builtin_entries};
use chrono::{TimeZone as _, Utc};
use rstest::rstest;

const LLAMA3_RENDERED: &str = "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\ns<|eot_id|><|start_header_id|>user<|end_header_id|>\n\nu<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\na<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n";

fn clock() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#).unwrap()
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

#[rstest]
#[case::generic_llama2_chat(
    "meta-llama/llama-2-13b-chat",
    "<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>"
)]
#[case::generic_llama3_instruct("meta-llama/llama-3-8b-instruct", LLAMA3_RENDERED)]
#[case::generic_falcon_instruct("tiiuae/falcon-7b-instruct", "suser:u\n\nassistant:a\n\n")]
#[case::generic_mpt_chat(
    "mosaicml/mpt-7b-chat",
    "<|im_start|>systems<|im_end|>\n<|im_start|>useru<|im_end|>\n<|im_start|>assistanta<|im_end|>\n"
)]
#[case::generic_codellama_instruct(
    "codellama/codellama-34b-instruct",
    "<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>"
)]
#[case::generic_wizardcoder(
    "wizardlm/wizardcoder-15b-v1.0",
    "s\n\n### Instruction:\nu\n\n### Response:\na\n\n"
)]
#[case::generic_phind(
    "phind/phind-codellama-34b-v2",
    "### System Prompt\ns\n\n### User Message\nu\n\n### Assistant\na\n\n"
)]
#[case::generic_together_llama2(
    "togethercomputer/llama-2-7b-chat",
    "<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>"
)]
#[case::generic_mythomax(
    "gryphe/mythomax-l2-13b",
    "<s>### Instruction:\ns\n\n### Instruction:\nu\n\n### Response:\na\n\n</s>"
)]
#[tokio::test]
async fn prompt_templates_prompt_template_tests_behavior(
    #[case] model: &str,
    #[case] expected: &str,
) {
    let output = fallback(
        &factory(),
        &model.to_lowercase(),
        model,
        &messages(),
        clock(),
    )
    .await
    .unwrap();
    assert_eq!(output, Output::Text(expected.to_owned()));
}

#[tokio::test]
async fn falcon_without_instruct_or_180b_chat_is_empty_output() {
    let output = fallback(
        &factory(),
        "tiiuae/falcon-7b",
        "tiiuae/falcon-7b",
        &messages(),
        clock(),
    )
    .await
    .unwrap();
    assert_eq!(output, Output::None);
}

#[tokio::test]
async fn mpt_without_chat_is_empty_output() {
    let output = fallback(
        &factory(),
        "mosaicml/mpt-7b",
        "mosaicml/mpt-7b",
        &messages(),
        clock(),
    )
    .await
    .unwrap();
    assert_eq!(output, Output::None);
}

#[tokio::test]
async fn codellama_without_instruct_is_empty_output() {
    let output = fallback(
        &factory(),
        "codellama/codellama-34b",
        "codellama/codellama-34b",
        &messages(),
        clock(),
    )
    .await
    .unwrap();
    assert_eq!(output, Output::None);
}

#[tokio::test]
async fn falls_back_to_default_pt_when_no_local_template_is_found() {
    let output = fallback(
        &factory(),
        "meta-llama/llama-3-8b",
        "meta-llama/llama-3-8b",
        &messages(),
        clock(),
    )
    .await
    .unwrap();
    assert_eq!(output, Output::Text("s u a".to_owned()));
}

#[tokio::test]
async fn llamaguard_renders_through_jinja() {
    let input: Vec<Message> =
        serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap();
    let output = fallback(
        &factory(),
        "meta-llama/llamaguard-7b",
        "meta-llama/llamaguard-7b",
        &input,
        clock(),
    )
    .await
    .unwrap();
    assert!(matches!(output, Output::Text(text) if text.contains("safety assessment")));
}
