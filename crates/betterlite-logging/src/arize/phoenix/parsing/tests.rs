#![cfg(test)]

use rstest::rstest;

use super::{ParsingError, parse_prompt_data};
use crate::arize::phoenix::records::{PromptTemplate, PromptVersionData, PromptVersionEnvelope};
use crate::arize::phoenix::tests::server::PROMPT_BODY;

fn version_data(text: &str) -> PromptVersionData {
    serde_json::from_str(text).unwrap()
}

fn parse(text: &str) -> Result<PromptTemplate, ParsingError> {
    parse_prompt_data(version_data(text), "v1")
}

fn invocation(parameters: &str) -> String {
    format!(r#"{{"invocation_parameters":{parameters}}}"#)
}

#[test]
fn parsing_extracts_messages_metadata_and_parameters_from_a_phoenix_response() {
    let envelope: PromptVersionEnvelope = serde_json::from_str(PROMPT_BODY).unwrap();
    let data = envelope.data.unwrap();
    let template = parse_prompt_data(data, "UHJvbXB0VmVyc2lvbjox").unwrap();
    assert_eq!(template.template_id, "UHJvbXB0VmVyc2lvbjox");
    assert_eq!(template.messages.len(), 2);
    assert_eq!(template.messages[0].role, "system");
    assert_eq!(
        template.messages[0].content[0].text,
        "You answer questions about {{ topic }}."
    );
    let metadata = template.metadata;
    assert_eq!(metadata.model_name.as_deref(), Some("gpt-4o"));
    assert_eq!(metadata.model_provider.as_deref(), Some("OPENAI"));
    assert_eq!(metadata.description, "answers questions");
    assert_eq!(metadata.template_type.as_deref(), Some("CHAT"));
    assert_eq!(metadata.template_format, "MUSTACHE");
    assert_eq!(metadata.temperature.unwrap().to_string(), "0.5");
    assert_eq!(metadata.max_tokens.unwrap().to_string(), "256");
    assert_eq!(metadata.invocation_parameters.len(), 2);
}

#[rstest]
#[case::openai_before_anthropic(
    r#"{"anthropic":{"temperature":0.9,"max_tokens":9},"openai":{"temperature":0.1,"max_tokens":1}}"#,
    "0.1",
    "1"
)]
#[case::anthropic_when_openai_is_absent(
    r#"{"type":"anthropic","anthropic":{"temperature":0.3,"max_tokens":2000}}"#,
    "0.3",
    "2000"
)]
#[case::first_nested_object(
    r#"{"type":"google","vertex":{"temperature":0.7,"max_tokens":64},"other":{"temperature":0.2,"max_tokens":8}}"#,
    "0.7",
    "64"
)]
fn parsing_selects_the_provider_block(
    #[case] parameters: &str,
    #[case] temperature: &str,
    #[case] max_tokens: &str,
) {
    let metadata = parse(&invocation(parameters)).unwrap().metadata;
    assert_eq!(metadata.temperature.unwrap().to_string(), temperature);
    assert_eq!(metadata.max_tokens.unwrap().to_string(), max_tokens);
}

#[rstest]
#[case::no_parameters("{}")]
#[case::only_scalars(r#"{"type":"openai","version":3}"#)]
#[case::block_without_the_keys(r#"{"openai":{"top_p":0.5}}"#)]
#[case::null_values(r#"{"openai":{"temperature":null,"max_tokens":null}}"#)]
fn parsing_leaves_temperature_and_max_tokens_absent(#[case] parameters: &str) {
    let metadata = parse(&invocation(parameters)).unwrap().metadata;
    assert_eq!(metadata.temperature, None);
    assert_eq!(metadata.max_tokens, None);
}

#[test]
fn parsing_applies_litellm_defaults_to_missing_and_null_fields() {
    let template = parse(
        r#"{"description":null,"template_format":"F","template":{"messages":[{"content":[{"text":"hi"}]},{"role":"assistant","content":null}]}}"#,
    )
    .unwrap();
    assert_eq!(template.messages[0].role, "user");
    assert_eq!(template.messages[0].content[0].kind, "");
    assert!(template.messages[1].content.is_empty());
    assert_eq!(template.metadata.description, "");
    assert_eq!(template.metadata.template_format, "F");
    let bare = parse("{}").unwrap();
    assert!(bare.messages.is_empty());
    assert_eq!(bare.metadata.template_format, "MUSTACHE");
    assert_eq!(bare.metadata.model_name, None);
}

#[rstest]
#[case::openai_not_an_object(
    r#"{"openai":"fast"}"#,
    ParsingError::ProviderParameters { provider: "openai".to_owned() }
)]
#[case::anthropic_null(
    r#"{"anthropic":null}"#,
    ParsingError::ProviderParameters { provider: "anthropic".to_owned() }
)]
#[case::temperature_text(r#"{"openai":{"temperature":"hot"}}"#, ParsingError::Temperature)]
#[case::max_tokens_text(r#"{"openai":{"max_tokens":"many"}}"#, ParsingError::MaxTokens)]
#[case::max_tokens_fractional(r#"{"openai":{"max_tokens":1.5}}"#, ParsingError::MaxTokens)]
fn parsing_rejects_parameters_of_the_wrong_type(
    #[case] parameters: &str,
    #[case] expected: ParsingError,
) {
    assert_eq!(parse(&invocation(parameters)).unwrap_err(), expected);
}

#[test]
fn parsing_rejects_content_that_is_not_a_list_of_parts() {
    let error = serde_json::from_str::<PromptVersionData>(
        r#"{"template":{"messages":[{"role":"user","content":"plain"}]}}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("invalid type: string"));
}
