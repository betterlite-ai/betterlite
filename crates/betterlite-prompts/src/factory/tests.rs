#![cfg(test)]

use super::{Output, prompt_factory};
use crate::builtin::Message;
use crate::error::FactoryError;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;

fn clock() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn messages(json_text: &str) -> Vec<Message> {
    serde_json::from_str(json_text).unwrap()
}

#[rstest]
#[case::factory_default(
    "gpt2",
    None,
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "s u a"
)]
#[case::factory_builtin(
    "anthropic.claude-v2",
    Some("bedrock"),
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a"
)]
fn prompt_templates_prompt_template_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] wire: &str,
    #[case] expected: &str,
) {
    let output = prompt_factory(model, &messages(wire), custom_llm_provider, clock()).unwrap();
    assert_eq!(output, Output::Text(expected.to_owned()));
}

#[test]
fn prompt_factory_dispatches_ollama() {
    let output = prompt_factory(
        "llama2",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        Some("ollama"),
        clock(),
    )
    .unwrap();
    assert!(matches!(output, Output::Ollama(_)));
}

#[rstest]
#[case::mistral("mistral-large", "mistral", "mistral_transform")]
#[case::perplexity("some-model", "perplexity", "perplexity")]
#[case::azure_text("some-model", "azure_text", "azure_text")]
#[case::watsonx("some-model", "watsonx", "watsonx")]
#[case::anthropic("claude-3-opus", "anthropic", "anthropic_messages")]
fn prompt_factory_delegates_installed_adapters(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] operation: &str,
) {
    let error = prompt_factory(
        model,
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        Some(provider),
        clock(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        FactoryError::Unsupported { operation: found } if found == operation
    ));
}

#[rstest]
#[case::bedrock_missing_local_template_raises(
    "bedrock/meta.llama3-70b-instruct-v1:0",
    Some("bedrock"),
    None
)]
#[case::generic_missing_local_template_falls_back(
    "meta-llama/llama-3-8b-instruct",
    None,
    Some("s u")
)]
fn prompt_factory_cases(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let input = messages(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#);
    let result = prompt_factory(model, &input, custom_llm_provider, clock());
    if let Some(text) = expected {
        assert_eq!(result.unwrap(), Output::Text(text.to_owned()));
        return;
    }
    let error = result.unwrap_err();
    assert!(matches!(error, FactoryError::NoChatTemplate));
    assert_eq!(error.to_string(), "No chat template found");
}
