#![cfg(test)]

use std::sync::Mutex;

use super::dispatch;
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
#[case::bedrock_llama3_instruct("meta.llama3-70b-instruct-v1:0", LLAMA3_RENDERED)]
#[tokio::test]
async fn prompt_templates_prompt_template_tests_behavior(
    #[case] model: &str,
    #[case] expected: &str,
) {
    let output = dispatch(&factory(), model, &messages(), clock())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output, Output::Text(expected.to_owned()));
}

#[tokio::test]
async fn unmatched_model_returns_none() {
    assert!(
        dispatch(&factory(), "some-other-model", &messages(), clock())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn llama2_without_chat_returns_none() {
    assert!(
        dispatch(&factory(), "meta.llama2-13b-v1", &messages(), clock())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn llama3_without_instruct_returns_none() {
    assert!(
        dispatch(
            &factory(),
            "meta.llama3-70b-chat-v1:0",
            &messages(),
            clock()
        )
        .await
        .is_none()
    );
}
