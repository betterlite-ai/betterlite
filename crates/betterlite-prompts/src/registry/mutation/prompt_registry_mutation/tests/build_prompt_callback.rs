#![cfg(test)]

use betterlite_models::prompt_spec::{PromptInfo, PromptSpec};
use indexmap::IndexMap;

use super::callback_build::build_prompt_callback;
use super::support::{api_key, client, prompt_spec};
use crate::registry::mutation::prompt_registry_mutation::error::PromptRegistryError;
use crate::registry::mutation::prompt_registry_mutation::methods::same_litellm_params;
use crate::tests::json_value;

fn timestamp(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .expect("the fixture timestamp is valid")
        .with_timezone(&chrono::Utc)
}

#[test]
fn returns_the_parsed_spec_and_the_initializer_client() {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let (parsed, callback) = build_prompt_callback(&prompt).expect("the prompt has a backend");
    assert_eq!(parsed.prompt_id, "greeting.v1");
    assert_eq!(callback, client(Some("greeting")));
}

#[test]
fn the_parsed_spec_copies_every_field_of_the_input() {
    let mut prompt = prompt_spec("greeting.v1", "production", "gitlab");
    prompt.litellm_params.api_base = Some("https://gitlab.example.test".to_owned());
    prompt.litellm_params.api_key = Some(api_key("secret"));
    prompt.litellm_params.dotprompt_content = Some("template".to_owned());
    prompt.litellm_params.provider_specific_query_params = Some(IndexMap::from([(
        "query".to_owned(),
        json_value("\"value\""),
    )]));
    prompt.litellm_params.extra.insert(
        "gitlab_config".to_owned(),
        json_value("{\"project\":\"group/repo\"}"),
    );
    prompt.prompt_info = PromptInfo {
        prompt_type: "db".to_owned(),
        environment: "staging".to_owned(),
        extra: IndexMap::from([("custom".to_owned(), json_value("1"))]),
    };
    prompt.created_at = Some(timestamp("2024-01-02T03:04:05Z"));
    prompt.updated_at = Some(timestamp("2024-02-03T04:05:06Z"));
    prompt.version = Some(4);
    prompt.created_by = Some("someone".to_owned());
    let (parsed, _callback) = build_prompt_callback(&prompt).expect("the prompt has a backend");
    assert_eq!(parsed.prompt_id, prompt.prompt_id);
    assert_eq!(parsed.environment, "production");
    assert_eq!(parsed.created_at, prompt.created_at);
    assert_eq!(parsed.updated_at, prompt.updated_at);
    assert_eq!(parsed.version, Some(4));
    assert_eq!(parsed.created_by, Some("someone".to_owned()));
    assert_eq!(
        parsed.litellm_params.api_base,
        Some("https://gitlab.example.test".to_owned())
    );
    assert!(same_litellm_params(
        &parsed.litellm_params,
        &prompt.litellm_params
    ));
    assert_eq!(parsed.prompt_info, prompt.prompt_info);
}

#[test]
fn an_unsupported_integration_is_an_error_naming_it() {
    let prompt = prompt_spec("greeting.v1", "development", "langfuse");
    let error = build_prompt_callback(&prompt).expect_err("langfuse has no initializer");
    assert!(matches!(
        &error,
        PromptRegistryError::UnsupportedIntegration { integration } if integration == "langfuse"
    ));
    assert_eq!(error.to_string(), "Unsupported prompt: langfuse");
}

#[test]
fn an_empty_integration_is_unsupported() {
    let prompt = prompt_spec("greeting.v1", "development", "");
    let error = build_prompt_callback(&prompt).expect_err("an empty integration has no backend");
    assert!(matches!(
        error,
        PromptRegistryError::UnsupportedIntegration { integration } if integration.is_empty()
    ));
}

#[test]
fn a_missing_prompt_integration_is_rejected_where_the_spec_is_read() {
    let body = r#"{"prompt_id":"greeting.v1","litellm_params":{}}"#;
    let error = serde_json::from_str::<PromptSpec>(body).expect_err("the integration is required");
    assert!(error.to_string().contains("prompt_integration"));
}

#[test]
fn an_absent_prompt_info_gets_the_config_default() {
    let body = r#"{"prompt_id":"greeting.v1","litellm_params":{"prompt_integration":"dotprompt","prompt_id":"repo/greeting"}}"#;
    let prompt: PromptSpec = serde_json::from_str(body).expect("the row parses");
    let (parsed, callback) = build_prompt_callback(&prompt).expect("dotprompt registers");
    assert_eq!(parsed.prompt_info.prompt_type, "config");
    assert_eq!(
        parsed.litellm_params.prompt_id,
        Some("repo/greeting".to_owned())
    );
    assert_eq!(callback, client(Some("repo/greeting")));
}

#[test]
fn the_initializer_receives_the_spec_params() {
    let mut prompt = prompt_spec("greeting.v1", "development", "bitbucket");
    prompt.litellm_params.prompt_id = Some("repo/prompt".to_owned());
    prompt.litellm_params.api_base = Some("https://bitbucket.example.test".to_owned());
    let (_parsed, callback) = build_prompt_callback(&prompt).expect("bitbucket registers");
    assert_eq!(callback, client(Some("repo/prompt")));
}
