#![cfg(test)]

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rstest::rstest;

use super::{apply_prompt_template, messages_to_prompt};
use crate::builtin::Message;
use crate::factory::PromptFactory;
use crate::factory::hub::{ChatTemplate, Hub, TokenizerConfig, TokenizerDocument, builtin_entries};

const ROLE_TEMPLATE: &str = "{% for message in messages %}<{{ message['role'] }}>{{ message['content'] }}</{{ message['role'] }}>{% endfor %}";

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn factory_with(model: &str, template: &str) -> PromptFactory {
    let factory = factory();
    factory.cache.lock().unwrap().insert(
        model.to_owned(),
        TokenizerConfig::Success(Some(TokenizerDocument {
            chat_template: ChatTemplate::Text(template.to_owned()),
            bos_token: None,
            eos_token: None,
        })),
    );
    factory
}

fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

#[rstest]
#[case::granite("watsonx/granite-chat-13b", "<|user|>\nhi\n<|assistant|>")]
#[case::ibm_mistral("watsonx/ibm-mistral-8x7b", "<s>[INST] hi [/INST]\n")]
#[case::ibm_mistral_before_mistralai("watsonx/ibm-mistralai/mixtral", "<s>[INST] hi [/INST]\n")]
#[case::granite_before_gpt_oss(
    "watsonx/granite-chat-openai/gpt-oss-120b",
    "<|user|>\nhi\n<|assistant|>"
)]
#[case::llama3(
    "watsonx/meta-llama/llama-3-instruct-8b",
    "<|begin_of_text|><|start_header_id|>user<|end_header_id|>\nhi<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n"
)]
#[tokio::test]
async fn prompt_templates_follow_the_pattern_order(#[case] model: &str, #[case] expected: &str) {
    let prompt = apply_prompt_template(
        &factory(),
        model,
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt.as_deref(), Some(expected));
}

#[rstest]
#[case::watsonx_prefix("watsonx/openai/gpt-oss-120b", "openai/gpt-oss-120b")]
#[case::watsonx_text_prefix("watsonx_text/openai/gpt-oss-120b", "openai/gpt-oss-120b")]
#[case::no_prefix("openai/gpt-oss-120b", "openai/gpt-oss-120b")]
#[tokio::test]
async fn prompt_templates_render_the_cached_hugging_face_template(
    #[case] model: &str,
    #[case] cached: &str,
) {
    let prompt = apply_prompt_template(
        &factory_with(cached, ROLE_TEMPLATE),
        model,
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt.as_deref(), Some("<user>hi</user>"));
}

#[tokio::test]
async fn prompt_templates_swallow_a_hugging_face_error() {
    let prompt = apply_prompt_template(
        &factory(),
        "watsonx/openai/gpt-oss-120b",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt, None);
}

#[tokio::test]
async fn prompt_templates_return_none_for_an_empty_render() {
    let prompt = apply_prompt_template(
        &factory_with("openai/gpt-oss-120b", "{{ '' }}"),
        "watsonx/openai/gpt-oss-120b",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt, None);
}

#[rstest]
#[case::unknown_model("unknown-model")]
#[case::empty_model("")]
#[tokio::test]
async fn prompt_templates_return_none_without_a_pattern(#[case] model: &str) {
    let prompt = apply_prompt_template(
        &factory(),
        model,
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt, None);
}

#[tokio::test]
async fn messages_to_prompt_uses_the_mistral_template_for_the_ibm_mistralai_provider() {
    let prompt = messages_to_prompt(
        &factory(),
        "ibm-mistralai/mixtral",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        "ibm-mistralai",
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt.as_deref(), Some("<s>[INST] hi [/INST]\n"));
}

#[tokio::test]
async fn messages_to_prompt_falls_back_to_the_lowercased_model() {
    let prompt = messages_to_prompt(
        &factory(),
        "WATSONX/Granite-Chat",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        "watsonx",
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt.as_deref(), Some("<|user|>\nhi\n<|assistant|>"));
}

#[tokio::test]
async fn messages_to_prompt_returns_none_when_no_template_matches() {
    let prompt = messages_to_prompt(
        &factory(),
        "unknown-model",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        "watsonx",
        now(),
    )
    .await
    .unwrap();
    assert_eq!(prompt, None);
}
