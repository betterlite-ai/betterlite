use rstest::rstest;

use super::support;
use crate::message_utilities::small_helpers::{
    EMPTY_USAGE, PromptTemplateRegistry, TokenUsage, get_first_chars_messages,
    register_prompt_template, should_run_mock_completion,
};

#[tokio::test]
async fn prompt_template_registers_the_full_and_the_resolved_model() {
    let mut registry = PromptTemplateRegistry::default();
    let template = support::template(
        r#"{"roles": {"user": {"pre_message": "[INST] ", "post_message": " [/INST]"}}, "initial_prompt_value": "hi", "final_prompt_value": "bye"}"#,
    );
    register_prompt_template(
        &mut registry,
        "openai/gpt-4o-mini",
        Some("gpt-4o-mini"),
        &template,
        None,
    );
    assert_eq!(
        registry.custom_prompt_dict.keys().collect::<Vec<_>>(),
        vec!["gpt-4o-mini", "openai/gpt-4o-mini"]
    );
    assert!(registry.known_tokenizer_config.is_empty());
}

#[tokio::test]
async fn prompt_template_registers_a_tokenizer_config_under_both_names() {
    let mut registry = PromptTemplateRegistry::default();
    let template =
        support::template(r#"{"roles": {}, "initial_prompt_value": "", "final_prompt_value": ""}"#);
    let config = support::value(r#"{"tokenizer": "x"}"#);
    let returned = register_prompt_template(
        &mut registry,
        "my-model",
        Some("my-model"),
        &template,
        Some(&config),
    );
    assert!(returned.is_empty());
    assert!(registry.custom_prompt_dict.is_empty());
    assert_eq!(
        registry.known_tokenizer_config.keys().collect::<Vec<_>>(),
        vec!["my-model"]
    );
    assert_eq!(
        registry
            .known_tokenizer_config
            .get("my-model")
            .map(|known| known.status.as_str()),
        Some("success")
    );
}

#[tokio::test]
async fn prompt_template_treats_an_empty_tokenizer_config_as_absent() {
    let mut registry = PromptTemplateRegistry::default();
    let template =
        support::template(r#"{"roles": {}, "initial_prompt_value": "", "final_prompt_value": ""}"#);
    let config = support::value("{}");
    register_prompt_template(&mut registry, "m", None, &template, Some(&config));
    assert_eq!(
        registry.custom_prompt_dict.keys().collect::<Vec<_>>(),
        vec!["m"]
    );
    assert!(registry.known_tokenizer_config.is_empty());
}

#[rstest]
#[case::object(
    r#"{"messages": [{"role": "user", "content": "hello"}]}"#,
    r#"[{"role":"user","content":"hello"}]"#
)]
#[case::missing("{}", "null")]
#[case::string(r#"{"messages": "abc"}"#, r#""abc""#)]
#[tokio::test]
async fn first_chars_messages_returns_the_json_text(#[case] input: &str, #[case] wanted: &str) {
    let kwargs = support::value(input);
    let text = get_first_chars_messages(&kwargs);
    let truncated: String = wanted.chars().take(100).collect();
    assert_eq!(text, truncated);
}

#[tokio::test]
async fn first_chars_messages_truncates_to_one_hundred_characters() {
    let kwargs = support::value(&format!(r#"{{"messages": "{}"}}"#, "x".repeat(200)));
    let text = get_first_chars_messages(&kwargs);
    assert_eq!(text.chars().count(), 100);
    assert_eq!(text, format!("\"{}", "x".repeat(99)));
}

#[tokio::test]
async fn first_chars_messages_returns_nothing_for_a_non_object() {
    assert_eq!(
        get_first_chars_messages(&support::value("5")),
        String::new()
    );
}

#[tokio::test]
async fn empty_usage_has_three_zero_counts() {
    assert_eq!(
        EMPTY_USAGE,
        TokenUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }
    );
    assert_eq!(
        EMPTY_USAGE.to_json(),
        support::value(
            r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "prompt_tokens_details": null, "completion_tokens_details": null}"#
        )
    );
}

#[rstest]
#[case::absent("null", "null", "null", false)]
#[case::zero("0", "0", "0", false)]
#[case::empty_string(r#""""#, r#""""#, r#""""#, false)]
#[case::empty_list("[]", "[]", "[]", false)]
#[case::false_value("false", "false", "false", false)]
#[case::timeout_set("null", "null", "\"x\"", true)]
#[case::zero_and_text("0", "[]", r#""0""#, true)]
#[tokio::test]
async fn mock_completion_runs_for_truthy_arguments(
    #[case] response: &str,
    #[case] tool_calls: &str,
    #[case] timeout: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        should_run_mock_completion(
            &support::value(response),
            &support::value(tool_calls),
            &support::value(timeout)
        ),
        expected
    );
}
