#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::PromptLiteLLMParams;
use indexmap::IndexMap;

use super::{git_ref, gitlab_config};
use crate::gitlab_prompt_compiler::error::InitializerError;

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

fn with_config(value: ContractJson) -> IndexMap<String, ContractJson> {
    IndexMap::from([("gitlab_config".to_owned(), value)])
}

#[test]
fn the_config_comes_from_the_extra_field() {
    let extra =
        with_config(serde_json::from_str(r#"{"project": "g/r", "access_token": "t"}"#).unwrap());

    let config = gitlab_config(&params(extra)).unwrap();

    assert!(config.project.is_some());
    assert!(config.access_token.is_some());
}

#[test]
fn an_absent_config_is_an_error() {
    let error = gitlab_config(&params(IndexMap::new())).unwrap_err();

    assert!(matches!(error, InitializerError::MissingConfig));
}

#[test]
fn a_null_config_is_an_error() {
    let error = gitlab_config(&params(with_config(ContractJson::Null))).unwrap_err();

    assert!(matches!(error, InitializerError::MissingConfig));
}

#[test]
fn an_empty_config_is_an_error() {
    let error =
        gitlab_config(&params(with_config(ContractJson::Object(IndexMap::new())))).unwrap_err();

    assert!(matches!(error, InitializerError::MissingConfig));
}

#[test]
fn a_config_that_is_not_a_mapping_is_an_error_that_names_the_field() {
    let error =
        gitlab_config(&params(with_config(ContractJson::Text("nope".to_owned())))).unwrap_err();

    assert!(matches!(error, InitializerError::Config(_)));
    assert!(error.to_string().contains("litellm_params.gitlab_config"));
}

#[test]
fn the_git_ref_comes_from_the_extra_field() {
    let extra = IndexMap::from([(
        "git_ref".to_owned(),
        ContractJson::Text("release/1".to_owned()),
    )]);

    assert_eq!(
        git_ref(&params(extra)).unwrap().as_deref(),
        Some("release/1")
    );
}

#[test]
fn an_absent_or_null_git_ref_is_none() {
    assert_eq!(git_ref(&params(IndexMap::new())).unwrap(), None);
    assert_eq!(
        git_ref(&params(IndexMap::from([(
            "git_ref".to_owned(),
            ContractJson::Null
        )])))
        .unwrap(),
        None
    );
}

#[test]
fn a_git_ref_that_is_not_text_is_an_error() {
    let extra = IndexMap::from([("git_ref".to_owned(), ContractJson::Bool(true))]);

    let error = git_ref(&params(extra)).unwrap_err();

    assert!(matches!(error, InitializerError::RefNotString));
}
