#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{AzureSearchSettings, Settings};

const BASE: &str = "model_list:\n  - model_name: foundry-vector\n    litellm_params:\n      model: azure_ai/text-embedding-3-small\n";

fn load_source(source: String, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(&source)
        .map(|config| config.sections.foundry_vector_stores)
}

fn load(extra_params: &str, settings: &str, variables: &[(&str, &str)]) -> Settings {
    let source = format!("{BASE}{extra_params}{settings}");
    load_source(source, variables).expect("the config should load")
}

fn deployment(settings: &Settings) -> &AzureSearchSettings {
    settings.deployments.first().expect("one deployment")
}

#[rstest]
#[case::loads_azure_search_service_name(0)]
#[case::loads_azure_search_vector_field(1)]
#[case::rejects_unrelated_key(2)]
fn foundry_vector_store_config_cases(#[case] case: u8) {
    match case {
        0 => loads_azure_search_service_name(),
        1 => loads_azure_search_vector_field(),
        _ => rejects_unrelated_key(),
    }
}

fn loads_azure_search_service_name() {
    assert_eq!(deployment(&load("", "", &[])).service_name, None);
    let explicit = load("      azure_search_service_name: deployment-service\n", "", &[]);
    assert_eq!(
        deployment(&explicit).service_name.as_deref(),
        Some("deployment-service")
    );
    let module = load(
        "",
        "litellm_settings:\n  azure_search_service_name: module-service\n",
        &[],
    );
    assert_eq!(
        deployment(&module).service_name.as_deref(),
        Some("module-service")
    );
    let both = load(
        "      azure_search_service_name: deployment-service\n",
        "litellm_settings:\n  azure_search_service_name: module-service\n",
        &[],
    );
    assert_eq!(
        deployment(&both).service_name.as_deref(),
        Some("deployment-service")
    );
    loads_azure_search_service_name_falls_back();
}

fn loads_azure_search_service_name_falls_back() {
    let empty = load(
        "      azure_search_service_name: \"\"\n",
        "litellm_settings:\n  azure_search_service_name: module-service\n",
        &[],
    );
    assert_eq!(
        deployment(&empty).service_name.as_deref(),
        Some("module-service")
    );
    let reference = load(
        "      azure_search_service_name: os.environ/FOUNDRY_SEARCH_SERVICE\n",
        "",
        &[("FOUNDRY_SEARCH_SERVICE", "referenced-service")],
    );
    assert_eq!(
        deployment(&reference).service_name.as_deref(),
        Some("referenced-service")
    );
}

fn loads_azure_search_vector_field() {
    assert_eq!(deployment(&load("", "", &[])).vector_field, "contentVector");
    let explicit = load("      azure_search_vector_field: customVector\n", "", &[]);
    assert_eq!(deployment(&explicit).vector_field, "customVector");
    let module = load(
        "",
        "litellm_settings:\n  azure_search_vector_field: moduleVector\n",
        &[],
    );
    assert_eq!(deployment(&module).vector_field, "moduleVector");
    let both = load(
        "      azure_search_vector_field: customVector\n",
        "litellm_settings:\n  azure_search_vector_field: moduleVector\n",
        &[],
    );
    assert_eq!(deployment(&both).vector_field, "customVector");
    loads_azure_search_vector_field_edges();
}

// an explicit empty vector field is used as given, while a null one falls through to the default
fn loads_azure_search_vector_field_edges() {
    let empty = load(
        "      azure_search_vector_field: \"\"\n",
        "litellm_settings:\n  azure_search_vector_field: moduleVector\n",
        &[],
    );
    assert_eq!(deployment(&empty).vector_field, "");
    let null = load("      azure_search_vector_field: null\n", "", &[]);
    assert_eq!(deployment(&null).vector_field, "contentVector");
    let reference = load(
        "      azure_search_vector_field: os.environ/FOUNDRY_VECTOR_FIELD\n",
        "",
        &[("FOUNDRY_VECTOR_FIELD", "referencedVector")],
    );
    assert_eq!(deployment(&reference).vector_field, "referencedVector");
}

fn rejects_unrelated_key() {
    let unrelated = format!("{BASE}litellm_settings:\n  foundry_vector_store_unrelated: 1\n");
    let error = load_source(unrelated, &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.foundry_vector_store_unrelated".to_owned()]
    );
    rejects_unrelated_deployment_key();
}

fn rejects_unrelated_deployment_key() {
    let unrelated = format!("{BASE}      foundry_vector_store_unrelated: 1\n");
    let error = load_source(unrelated, &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["model_list.0.litellm_params.foundry_vector_store_unrelated".to_owned()]
    );
}
