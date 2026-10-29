#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;

#[rstest]
#[case::custom_prompt(
    r#"{"mode":"custom_prompt","template":{"roles":{"user":{"pre_message":"[INST] ","post_message":" [/INST]"}},"initial_prompt_value":"<s>","final_prompt_value":"</s>","bos_token":"","eos_token":""},"messages":[{"role":"user","content":"hi"}]}"#,
    "<s>[INST] hi [/INST]</s>"
)]
#[case::default_response_schema_prompt(
    r#"{"mode":"default_response_schema_prompt","response_schema":{"type":"object"}}"#,
    "Use this JSON schema: \n    ```json \n    {'type': 'object'}\n    ```"
)]
#[case::response_schema_prompt_without_template(
    r#"{"mode":"response_schema_prompt","response_schema":{"type":"object"}}"#,
    "Use this JSON schema: \n    ```json \n    {'type': 'object'}\n    ```"
)]
#[case::factory_default(
    r#"{"mode":"factory","model":"gpt2","messages":[{"role":"user","content":"hi"}]}"#,
    "hi"
)]
fn prompt_factory_behavior_tests_behavior(#[case] wire: &str, #[case] expected: &str) {
    let input: Input = serde_json::from_str(wire).unwrap();
    assert_eq!(
        Op::run(&input).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn function_call_prompt_adds_a_system_message() {
    let input: Input = serde_json::from_str(
        r#"{"mode":"function_call_prompt","messages":[{"role":"user","content":"hi"}],"functions":[{"name":"lookup"}]}"#,
    )
    .unwrap();
    let output = Op::run(&input).unwrap();
    let messages = output.as_array().unwrap();
    assert_eq!(messages.len(), 2);
}

#[test]
fn bedrock_missing_local_template_raises_an_exception() {
    let input: Input = serde_json::from_str(
        r#"{"mode":"factory","model":"meta.llama3-70b-instruct-v1:0","custom_llm_provider":"bedrock","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "Exception");
}

#[test]
fn generic_missing_local_template_falls_back_to_default_pt() {
    let input: Input = serde_json::from_str(
        r#"{"mode":"factory","model":"meta-llama/llama-3-8b-instruct","messages":[{"role":"system","content":"s"},{"role":"user","content":"u"}]}"#,
    )
    .unwrap();
    assert_eq!(
        Op::run(&input).unwrap(),
        serde_json::to_value("s u").unwrap()
    );
}
