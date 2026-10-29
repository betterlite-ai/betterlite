#![cfg(test)]

use super::fallback;
use crate::builtin::Message;
use crate::factory::Output;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;

fn clock() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#).unwrap()
}

#[rstest]
#[case::generic_llama2_chat(
    "meta-llama/llama-2-13b-chat",
    "<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>"
)]
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
fn prompt_templates_prompt_template_tests_behavior(#[case] model: &str, #[case] expected: &str) {
    let output = fallback(&model.to_lowercase(), &messages(), clock()).unwrap();
    assert_eq!(output, Output::Text(expected.to_owned()));
}

#[test]
fn falcon_without_instruct_or_180b_chat_is_empty_output() {
    let output = fallback("tiiuae/falcon-7b", &messages(), clock()).unwrap();
    assert_eq!(output, Output::None);
}

#[test]
fn mpt_without_chat_is_empty_output() {
    let output = fallback("mosaicml/mpt-7b", &messages(), clock()).unwrap();
    assert_eq!(output, Output::None);
}

#[test]
fn codellama_without_instruct_is_empty_output() {
    let output = fallback("codellama/codellama-34b", &messages(), clock()).unwrap();
    assert_eq!(output, Output::None);
}

#[test]
fn falls_back_to_default_pt_when_no_local_template_is_found() {
    let output = fallback("meta-llama/llama-3-8b-instruct", &messages(), clock()).unwrap();
    assert_eq!(output, Output::Text("s u a".to_owned()));
}

#[test]
fn llamaguard_renders_through_jinja() {
    let input: Vec<Message> =
        serde_json::from_str(r#"[{"role":"user","content":"hello"}]"#).unwrap();
    let output = fallback("meta-llama/llamaguard-7b", &input, clock()).unwrap();
    assert!(matches!(output, Output::Text(text) if text.contains("safety assessment")));
}
