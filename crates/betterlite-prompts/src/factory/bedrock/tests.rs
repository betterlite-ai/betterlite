#![cfg(test)]

use super::dispatch;
use crate::builtin::Message;
use crate::error::FactoryError;
use crate::factory::Output;
use rstest::rstest;

fn messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#).unwrap()
}

#[rstest]
#[case::bedrock_titan(
    "amazon.titan-text-express-v1",
    "\n\nUser: <admin>s</admin>\n\nUser: u\n\nBot: a"
)]
#[case::bedrock_claude(
    "anthropic.claude-v2",
    "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"
)]
#[case::bedrock_claude_2_1("anthropic.claude-v2:1", "s\n\nHuman: u\n\nAssistant: a")]
#[case::bedrock_mistral(
    "mistral.mixtral-8x7b-instruct",
    "<s>[INST] \ns [/INST]\n[INST] u [/INST]\n a</s> "
)]
#[case::bedrock_llama2_chat(
    "meta.llama2-13b-chat",
    "<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>"
)]
fn prompt_templates_prompt_template_tests_behavior(#[case] model: &str, #[case] expected: &str) {
    let output = dispatch(model, &messages()).unwrap().unwrap();
    assert_eq!(output, Output::Text(expected.to_owned()));
}

#[test]
fn llama3_instruct_has_no_fallback() {
    let error = dispatch("meta.llama3-70b-instruct-v1:0", &messages())
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, FactoryError::NoChatTemplate));
}

#[test]
fn unmatched_model_returns_none() {
    assert!(dispatch("some-other-model", &messages()).is_none());
}
