#![cfg(test)]

use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{RecordingCallbacks, api_key, params, prompt_spec, registry_with};
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::mutation::prompt_registry_mutation::methods::same_litellm_params;
use crate::tests::json_value;

fn change_prompt_id(params: &mut PromptLiteLLMParams) {
    params.prompt_id = Some("other-id".to_owned());
}

fn change_integration(params: &mut PromptLiteLLMParams) {
    params.prompt_integration = "gitlab".to_owned();
}

fn change_api_base(params: &mut PromptLiteLLMParams) {
    params.api_base = Some("https://other.example.test".to_owned());
}

fn change_api_key(params: &mut PromptLiteLLMParams) {
    params.api_key = Some(api_key("secret"));
}

fn change_query_params(params: &mut PromptLiteLLMParams) {
    params.provider_specific_query_params = Some(IndexMap::from([(
        "query".to_owned(),
        json_value("\"value\""),
    )]));
}

fn change_ignore_model(params: &mut PromptLiteLLMParams) {
    params.ignore_model = true;
}

fn change_ignore_optional(params: &mut PromptLiteLLMParams) {
    params.ignore_optional_params = true;
}

fn change_dotprompt_content(params: &mut PromptLiteLLMParams) {
    params.dotprompt_content = Some("template".to_owned());
}

fn change_extra(params: &mut PromptLiteLLMParams) {
    params.extra.insert("custom".to_owned(), json_value("true"));
}

fn change_prompt_type(info: &mut PromptInfo) {
    info.prompt_type = "db".to_owned();
}

fn change_info_environment(info: &mut PromptInfo) {
    info.environment = "production".to_owned();
}

fn change_info_extra(info: &mut PromptInfo) {
    info.extra.insert("custom".to_owned(), json_value("1"));
}

fn assert_reload_of_changed_row(change: impl FnOnce(&mut PromptSpec)) {
    let prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    change(&mut row);
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("the differing row reloads");
    assert_eq!(callbacks.removed().len(), 1);
    assert_eq!(callbacks.added().len(), 1);
}

#[rstest]
#[case::prompt_id(change_prompt_id)]
#[case::integration(change_integration)]
#[case::api_base(change_api_base)]
#[case::api_key(change_api_key)]
#[case::query_params(change_query_params)]
#[case::ignore_model(change_ignore_model)]
#[case::ignore_optional_params(change_ignore_optional)]
#[case::dotprompt_content(change_dotprompt_content)]
#[case::extra(change_extra)]
fn a_differing_litellm_param_reloads_the_prompt(#[case] mutate: fn(&mut PromptLiteLLMParams)) {
    assert_reload_of_changed_row(|row| mutate(&mut row.litellm_params));
}

#[rstest]
#[case::prompt_type(change_prompt_type)]
#[case::environment(change_info_environment)]
#[case::extra(change_info_extra)]
fn a_differing_prompt_info_reloads_the_prompt(#[case] mutate: fn(&mut PromptInfo)) {
    assert_reload_of_changed_row(|row| mutate(&mut row.prompt_info));
}

#[test]
fn an_equal_row_returns_the_stored_prompt_without_reloading() {
    let mut prompt = prompt_spec("greeting.v1", "development", "dotprompt");
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
    prompt
        .prompt_info
        .extra
        .insert("custom".to_owned(), json_value("1"));
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.created_at = Some(timestamp("2024-01-02T03:04:05Z"));
    row.updated_at = Some(timestamp("2024-02-03T04:05:06Z"));
    row.version = Some(9);
    row.created_by = Some("someone-else".to_owned());
    let returned = registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("an equal row skips the reload");
    assert!(callbacks.calls.is_empty());
    assert_eq!(returned.version, None);
    assert_eq!(returned.created_at, None);
    assert_eq!(returned.created_by, None);
}

#[test]
fn the_same_api_key_value_counts_as_equal() {
    let mut prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    prompt.litellm_params.api_key = Some(api_key("secret"));
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.litellm_params.api_key = Some(api_key("secret"));
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("the row matches");
    assert!(callbacks.calls.is_empty());
}

#[test]
fn a_different_api_key_value_reloads_the_prompt() {
    let mut prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    prompt.litellm_params.api_key = Some(api_key("secret"));
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.litellm_params.api_key = Some(api_key("other"));
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("the differing row reloads");
    assert_eq!(callbacks.removed().len(), 1);
}

#[test]
fn a_disappearing_api_key_reloads_the_prompt() {
    let mut prompt = prompt_spec("greeting.v1", "development", "dotprompt");
    prompt.litellm_params.api_key = Some(api_key("secret"));
    let mut callbacks = RecordingCallbacks::default();
    let mut registry = registry_with(&prompt, &mut callbacks);
    let mut row = prompt.clone();
    row.litellm_params.api_key = None;
    registry
        .sync_prompt_from_db(&row, &mut callbacks)
        .expect("the differing row reloads");
    assert_eq!(callbacks.removed().len(), 1);
}

#[test]
fn independently_built_params_compare_equal() {
    assert!(same_litellm_params(
        &params("dotprompt"),
        &params("dotprompt")
    ));
}

#[test]
fn a_different_integration_compares_unequal() {
    assert!(!same_litellm_params(
        &params("dotprompt"),
        &params("gitlab")
    ));
}

fn timestamp(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .expect("the fixture timestamp is valid")
        .with_timezone(&chrono::Utc)
}
