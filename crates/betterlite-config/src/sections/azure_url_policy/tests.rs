#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_url_policy::{AzureCredentials, Settings};

const BASE: &str =
    "model_list:\n  - model_name: azure-urls\n    litellm_params:\n      model: azure/gpt-4o\n";

fn load_source(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.azure_url_policy)
}

fn load(extra: &str, variables: &[(&str, &str)]) -> Settings {
    load_source(&format!("{BASE}{extra}"), variables).unwrap()
}

fn deployment(settings: &Settings) -> &AzureCredentials {
    settings.deployments.first().unwrap()
}

fn api_key(settings: &Settings) -> Option<String> {
    deployment(settings)
        .api_key
        .as_ref()
        .map(|value| value.expose_secret().to_owned())
}

fn base_from(extra: &str, variables: &[(&str, &str)]) -> Option<String> {
    deployment(&load(extra, variables)).api_base.clone()
}

fn version_from(extra: &str, variables: &[(&str, &str)]) -> Option<String> {
    deployment(&load(extra, variables)).api_version.clone()
}

#[rstest]
#[case::loads_azure_api_base(0)]
#[case::loads_azure_openai_api_key(1)]
#[case::loads_azure_api_key(2)]
#[case::loads_azure_api_version(3)]
#[case::rejects_unrelated_key(4)]
fn azure_urls_config_cases(#[case] case: u8) {
    match case {
        0 => loads_azure_api_base(),
        1 => loads_azure_openai_api_key(),
        2 => loads_azure_api_key(),
        3 => loads_azure_api_version(),
        _ => rejects_unrelated_key(),
    }
}

fn loads_azure_api_base() {
    assert_eq!(base_from("", &[]), None);
    assert_eq!(
        base_from(
            "      api_base: https://deployment.example\n",
            &[("AZURE_API_BASE", "https://environment.example")],
        )
        .as_deref(),
        Some("https://deployment.example")
    );
    let module = "litellm_settings:\n  api_base: https://module.example\n";
    assert_eq!(
        base_from(module, &[]).as_deref(),
        Some("https://module.example")
    );
    assert_eq!(
        base_from(module, &[("AZURE_API_BASE", "https://environment.example")]).as_deref(),
        Some("https://module.example")
    );
    assert_eq!(base_from("      api_base: \"\"\n", &[]), None);
    assert_eq!(
        base_from("", &[("AZURE_API_BASE", "https://environment.example")]).as_deref(),
        Some("https://environment.example")
    );
    assert_eq!(
        base_from("", &[("AZURE_API_BASE", "")]).as_deref(),
        Some("")
    );
    assert_eq!(
        base_from(
            "      api_base: os.environ/AZURE_API_BASE\n",
            &[("AZURE_API_BASE", "https://environment.example")],
        )
        .as_deref(),
        Some("https://environment.example")
    );
}

fn loads_azure_openai_api_key() {
    assert_eq!(api_key(&load("", &[])), None);
    assert_eq!(
        api_key(&load("", &[("AZURE_OPENAI_API_KEY", "environment-key")])).as_deref(),
        Some("environment-key")
    );
    assert_eq!(
        api_key(&load(
            "      api_key: deployment-key\n",
            &[("AZURE_OPENAI_API_KEY", "environment-key")],
        ))
        .as_deref(),
        Some("deployment-key")
    );
    assert_eq!(
        api_key(&load(
            "litellm_settings:\n  api_key: module-key\n",
            &[("AZURE_OPENAI_API_KEY", "environment-key")],
        ))
        .as_deref(),
        Some("module-key")
    );
    assert_eq!(
        api_key(&load(
            "litellm_settings:\n  azure_key: azure-key\n",
            &[("AZURE_OPENAI_API_KEY", "environment-key")],
        ))
        .as_deref(),
        Some("azure-key")
    );
    let settings = load("      api_key: fixture-secret\n", &[]);
    assert!(!format!("{settings:?}").contains("fixture-secret"));
    assert_eq!(
        settings.redact_known("echo fixture-secret back"),
        "echo [redacted] back"
    );
}

fn loads_azure_api_key() {
    assert_eq!(
        api_key(&load("", &[("AZURE_API_KEY", "last-key")])).as_deref(),
        Some("last-key")
    );
    assert_eq!(
        api_key(&load(
            "",
            &[
                ("AZURE_OPENAI_API_KEY", "openai-key"),
                ("AZURE_API_KEY", "last-key"),
            ],
        ))
        .as_deref(),
        Some("openai-key")
    );
    assert_eq!(
        api_key(&load(
            "",
            &[("AZURE_OPENAI_API_KEY", ""), ("AZURE_API_KEY", "last-key")],
        ))
        .as_deref(),
        Some("last-key")
    );
    assert_eq!(
        api_key(&load(
            "      api_key: os.environ/AZURE_API_KEY\n",
            &[("AZURE_API_KEY", "last-key")],
        ))
        .as_deref(),
        Some("last-key")
    );
}

fn loads_azure_api_version() {
    assert_eq!(version_from("", &[]), None);
    assert_eq!(
        version_from(
            "      api_version: 2024-02-01\n",
            &[("AZURE_API_VERSION", "2025-02-01-preview")],
        )
        .as_deref(),
        Some("2024-02-01")
    );
    let module = "litellm_settings:\n  api_version: module-version\n";
    assert_eq!(
        version_from(module, &[("AZURE_API_VERSION", "environment-version")]).as_deref(),
        Some("module-version")
    );
    assert_eq!(version_from(module, &[]).as_deref(), Some("module-version"));
    assert_eq!(version_from("      api_version: \"\"\n", &[]), None);
    assert_eq!(
        version_from("", &[("AZURE_API_VERSION", "environment-version")]).as_deref(),
        Some("environment-version")
    );
    assert_eq!(
        version_from("", &[("AZURE_API_VERSION", "")]).as_deref(),
        Some("")
    );
}

fn rejects_unrelated_key() {
    let error = load_source("litellm_settings:\n  azure_urls_unrelated_key: 1\n", &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_urls_unrelated_key".to_owned()]
    );
}
