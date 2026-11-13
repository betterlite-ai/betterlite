#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;

use super::prompt_initializer;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::tests::FakeTransport;
use crate::gitlab_prompt_compiler::error::InitializerError;
use crate::gitlab_templates::error::ManagerError;

fn params(extra: IndexMap<String, ContractJson>) -> PromptLiteLLMParams {
    PromptLiteLLMParams {
        prompt_id: None,
        prompt_integration: "gitlab".to_owned(),
        api_base: None,
        api_key: None,
        provider_specific_query_params: None,
        ignore_model: false,
        ignore_optional_params: false,
        dotprompt_content: None,
        extra,
    }
}

fn spec(prompt_id: &str, extra: IndexMap<String, ContractJson>) -> PromptSpec {
    PromptSpec {
        prompt_id: prompt_id.to_owned(),
        litellm_params: params(extra),
        prompt_info: PromptInfo::default(),
        created_at: None,
        updated_at: None,
        version: None,
        environment: "development".to_owned(),
        created_by: None,
    }
}

fn extra(config_json: &str) -> IndexMap<String, ContractJson> {
    IndexMap::from([(
        "gitlab_config".to_owned(),
        serde_json::from_str(config_json).unwrap(),
    )])
}

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

#[tokio::test]
async fn the_initializer_builds_a_manager_from_the_config() {
    let spec = spec("chat/hi", extra(CONFIG));

    let manager = prompt_initializer(&spec.litellm_params, &spec, FakeTransport::default())
        .await
        .unwrap();

    assert_eq!(manager.prompt_id.as_deref(), Some("chat/hi"));
    assert_eq!(manager.ref_override, None);
    assert_eq!(
        manager.gitlab_config.prompts_path.as_deref(),
        Some("prompts")
    );
}

#[tokio::test]
async fn the_initializer_uses_the_outer_prompt_id_and_the_git_ref() {
    let mut extra = extra(CONFIG);
    extra.insert("git_ref".to_owned(), ContractJson::Text("v2".to_owned()));
    let spec = spec("chat/hi", extra);

    let manager = prompt_initializer(&spec.litellm_params, &spec, FakeTransport::default())
        .await
        .unwrap();

    assert_eq!(manager.prompt_id.as_deref(), Some("chat/hi"));
    assert_eq!(manager.ref_override.as_deref(), Some("v2"));
}

#[tokio::test]
async fn a_missing_gitlab_config_is_an_error() {
    let spec = spec("chat/hi", IndexMap::new());

    let error = prompt_initializer(&spec.litellm_params, &spec, FakeTransport::default())
        .await
        .err()
        .unwrap();

    assert!(matches!(error, InitializerError::MissingConfig));
    assert_eq!(
        error.to_string(),
        "gitlab_config is required for gitlab prompt integration"
    );
}

#[tokio::test]
async fn a_git_ref_that_is_not_text_is_an_error() {
    let mut extra = extra(CONFIG);
    extra.insert(
        "git_ref".to_owned(),
        ContractJson::Number(serde_json::Number::from(4_i32)),
    );
    let spec = spec("chat/hi", extra);

    let error = prompt_initializer(&spec.litellm_params, &spec, FakeTransport::default())
        .await
        .err()
        .unwrap();

    assert!(matches!(error, InitializerError::RefNotString));
}

#[tokio::test]
async fn a_config_without_credentials_is_an_error_naming_the_client() {
    let spec = spec("chat/hi", extra(r#"{"project": "g/r"}"#));

    let error = prompt_initializer(&spec.litellm_params, &spec, FakeTransport::default())
        .await
        .err()
        .unwrap();

    assert!(matches!(
        error,
        InitializerError::Manager(ManagerError::Client(GitLabClientError::MissingCredentials))
    ));
}
