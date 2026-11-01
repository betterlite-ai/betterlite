#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;

use super::{DotpromptInitializerError, prompt_initializer};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;

fn litellm_params(
    prompt_id: Option<&str>,
    dotprompt_content: Option<&str>,
    extra: IndexMap<String, ContractJson>,
) -> PromptLiteLLMParams {
    PromptLiteLLMParams {
        prompt_id: prompt_id.map(str::to_owned),
        prompt_integration: "dotprompt".to_owned(),
        api_base: None,
        api_key: None,
        provider_specific_query_params: None,
        ignore_model: false,
        ignore_optional_params: false,
        dotprompt_content: dotprompt_content.map(str::to_owned),
        extra,
    }
}

fn prompt_spec(prompt_id: &str, litellm_params: PromptLiteLLMParams) -> PromptSpec {
    PromptSpec {
        prompt_id: prompt_id.to_owned(),
        litellm_params,
        prompt_info: PromptInfo::default(),
        created_at: None,
        updated_at: None,
        version: None,
        environment: "development".to_owned(),
        created_by: None,
    }
}

#[test]
fn rejects_prompt_directory_with_the_exact_message() {
    let mut extra = IndexMap::new();
    extra.insert(
        "prompt_directory".to_owned(),
        ContractJson::Text("/some/dir".to_owned()),
    );
    let params = litellm_params(None, None, extra);
    let spec = prompt_spec("y.v2", params);

    let error = prompt_initializer(
        &spec.litellm_params,
        &spec,
        &GlobalPromptDirectory::default(),
    )
    .expect_err("prompt_directory is rejected");

    assert!(matches!(
        error,
        DotpromptInitializerError::PromptDirectorySet
    ));
    assert_eq!(
        error.to_string(),
        "Cannot set prompt_directory when working with prompt_initializer. Needs to be a specific dotprompt file"
    );
}

#[test]
fn uses_dotprompt_content_from_the_database_when_data_and_file_are_absent() {
    let params = litellm_params(None, Some("User: hi"), IndexMap::new());
    let spec = prompt_spec("greeting", params);

    let manager = prompt_initializer(
        &spec.litellm_params,
        &spec,
        &GlobalPromptDirectory::default(),
    )
    .expect("dotprompt_content alone is a valid source");

    assert!(manager.should_run_prompt_management(Some("greeting")));
}

#[test]
fn the_nested_prompt_id_takes_precedence_over_the_registry_id() {
    let params = litellm_params(Some("x.v1"), Some("User: hi"), IndexMap::new());
    let spec = prompt_spec("y.v2", params);

    let manager = prompt_initializer(
        &spec.litellm_params,
        &spec,
        &GlobalPromptDirectory::default(),
    )
    .expect("dotprompt_content alone is a valid source");

    assert!(manager.should_run_prompt_management(Some("x.v1")));
    assert!(!manager.should_run_prompt_management(Some("y.v2")));
}

#[test]
fn falls_back_to_the_version_stripped_registry_id_when_no_nested_id_is_given() {
    let params = litellm_params(None, Some("User: hi"), IndexMap::new());
    let spec = prompt_spec("greeting.v3", params);

    let manager = prompt_initializer(
        &spec.litellm_params,
        &spec,
        &GlobalPromptDirectory::default(),
    )
    .expect("dotprompt_content alone is a valid source");

    assert!(manager.should_run_prompt_management(Some("greeting")));
}

#[test]
fn extra_prompt_data_is_loaded_when_dotprompt_content_is_absent() {
    let mut record = IndexMap::new();
    record.insert(
        "content".to_owned(),
        ContractJson::Text("User: hi".to_owned()),
    );
    let mut prompt_data = IndexMap::new();
    prompt_data.insert("greeting".to_owned(), ContractJson::Object(record));
    let mut extra = IndexMap::new();
    extra.insert("prompt_data".to_owned(), ContractJson::Object(prompt_data));

    let params = litellm_params(None, None, extra);
    let spec = prompt_spec("greeting", params);

    let manager = prompt_initializer(
        &spec.litellm_params,
        &spec,
        &GlobalPromptDirectory::default(),
    )
    .expect("extra.prompt_data alone is a valid source");

    assert!(manager.should_run_prompt_management(Some("greeting")));
}

#[test]
fn uses_the_registry_id_directly_when_it_has_no_version_suffix() {
    let params = litellm_params(None, Some("User: hi"), IndexMap::new());
    let spec = prompt_spec("greeting", params);

    let manager = prompt_initializer(
        &spec.litellm_params,
        &spec,
        &GlobalPromptDirectory::default(),
    )
    .expect("dotprompt_content alone is a valid source");

    assert!(manager.should_run_prompt_management(Some("greeting")));
}
