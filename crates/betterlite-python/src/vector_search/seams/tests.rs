#![cfg(test)]

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_search_execution::config::Params;
use betterlite_core::api::vector_store_http_write::config::WriteParams;
use betterlite_core::api::vector_store_search::service::{
    OptionalParamFilters as _, VectorStoreConfigs as _,
};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::{ExposeSecret, SecretString};

use super::{Configs, Filters, globals};

#[test]
fn create_params_keeps_the_declared_non_null_fields() {
    let given = Params::from([
        (String::from("name"), Json::Text(String::from("notes"))),
        (String::from("metadata"), Json::Null),
        (String::from("model"), Json::Text(String::from("gpt"))),
    ]);
    let filtered = Filters.create_params(&given);
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered.get("name"),
        Some(&Json::Text(String::from("notes")))
    );
}

#[test]
fn search_params_keeps_the_declared_non_null_fields() {
    let given = Params::from([
        (String::from("filters"), Json::Null),
        (String::from("max_num_results"), Json::Bool(true)),
        (String::from("name"), Json::Bool(true)),
    ]);
    let filtered = Filters.search_params(&given, VectorStoresConfigTag::OpenAI);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered.get("max_num_results"), Some(&Json::Bool(true)));
}

#[test]
fn configs_serve_openai_and_refuse_the_other_tags() {
    let configs = Configs::new(
        OpenAiCredentialsSettings::default(),
        LiteLlmGlobals::default(),
        None,
    );
    assert!(
        configs
            .create_config(VectorStoresConfigTag::OpenAI)
            .is_some()
    );
    assert!(
        configs
            .create_config(VectorStoresConfigTag::AzureOpenAI)
            .is_none()
    );
    assert!(
        configs
            .search_config(VectorStoresConfigTag::OpenAI)
            .is_some()
    );
    assert!(
        configs
            .search_config(VectorStoresConfigTag::AzureOpenAI)
            .is_none()
    );
}

#[test]
fn configs_resolve_the_module_api_base_globally() {
    let module_globals = LiteLlmGlobals {
        api_base: Some(String::from("http://127.0.0.1:9/v1")),
        api_key: None,
        openai_key: None,
        organization: None,
    };
    let configs = Configs::new(OpenAiCredentialsSettings::default(), module_globals, None);
    let create = configs
        .create_config(VectorStoresConfigTag::OpenAI)
        .expect("openai has a create config");
    let create_url = create
        .provider()
        .complete_url(&WriteParams::default())
        .expect("the module api base resolves");
    assert_eq!(create_url, "http://127.0.0.1:9/v1/vector_stores");
    let search = configs
        .search_config(VectorStoresConfigTag::OpenAI)
        .expect("openai has a search config");
    let search_url = search
        .complete_url(&Params::new())
        .expect("the module api base resolves");
    assert_eq!(search_url, "http://127.0.0.1:9/v1/vector_stores");
}

#[test]
fn globals_copies_every_credential_field() {
    let source = LiteLlmGlobals {
        api_base: Some(String::from("http://127.0.0.1:9/v1")),
        api_key: Some(SecretString::from("sk-key")),
        openai_key: None,
        organization: Some(String::from("team")),
    };
    let copied = globals(&source);
    assert_eq!(copied.api_base, source.api_base);
    assert_eq!(copied.organization, source.organization);
    assert_eq!(
        copied.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-key")
    );
    assert!(copied.openai_key.is_none());
}
