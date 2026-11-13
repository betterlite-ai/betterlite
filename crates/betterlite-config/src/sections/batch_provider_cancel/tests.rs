#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::batch_provider_cancel::Settings;

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(yaml)?;
    Ok(config.sections.batch_provider_cancel)
}

fn settings(yaml: &str, environment: &[(&str, &str)]) -> Settings {
    load(yaml, environment).expect("settings load")
}

fn snapshot(settings: &Settings) -> BTreeMap<&'static str, Option<String>> {
    BTreeMap::from([
        ("litellm_settings.api_base", settings.api_base.clone()),
        ("env.OPENAI_BASE_URL", settings.openai_base_url.clone()),
        ("env.OPENAI_API_BASE", settings.openai_api_base.clone()),
        (
            "litellm_settings.organization",
            settings.organization.clone(),
        ),
        (
            "env.OPENAI_ORGANIZATION",
            settings.openai_organization.clone(),
        ),
        (
            "litellm_settings.api_key",
            settings
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().clone()),
        ),
        (
            "litellm_settings.openai_key",
            settings
                .openai_key
                .as_ref()
                .map(|key| key.expose_secret().clone()),
        ),
        (
            "env.OPENAI_API_KEY",
            settings
                .openai_api_key
                .as_ref()
                .map(|key| key.expose_secret().clone()),
        ),
        ("env.AZURE_API_BASE", settings.azure_api_base.clone()),
        ("litellm_settings.api_version", settings.api_version.clone()),
        ("env.AZURE_API_VERSION", settings.azure_api_version.clone()),
        (
            "litellm_settings.azure_key",
            settings
                .azure_key
                .as_ref()
                .map(|key| key.expose_secret().clone()),
        ),
        (
            "env.AZURE_OPENAI_API_KEY",
            settings
                .azure_openai_api_key
                .as_ref()
                .map(|key| key.expose_secret().clone()),
        ),
        (
            "env.AZURE_API_KEY",
            settings
                .azure_api_key
                .as_ref()
                .map(|key| key.expose_secret().clone()),
        ),
        (
            "env.AZURE_AD_TOKEN",
            settings
                .azure_ad_token
                .as_ref()
                .map(|token| token.expose_secret().clone()),
        ),
        (
            "litellm_settings.vertex_project",
            settings.vertex_project.clone(),
        ),
        ("env.VERTEXAI_PROJECT", settings.vertexai_project.clone()),
        (
            "litellm_settings.vertex_location",
            settings.vertex_location.clone(),
        ),
        ("env.VERTEXAI_LOCATION", settings.vertexai_location.clone()),
        (
            "env.VERTEXAI_CREDENTIALS",
            settings
                .vertexai_credentials
                .as_ref()
                .map(|token| token.expose_secret().clone()),
        ),
    ])
}

fn value(settings: &Settings, key: &str) -> Option<String> {
    snapshot(settings).remove(key).flatten()
}

#[test]
fn defaults_are_unset() {
    let settings = settings("{}\n", &[]);
    assert!(snapshot(&settings).values().all(Option::is_none));
}

#[rstest]
#[case::litellm_settings_api_base("litellm_settings:\n  api_base: https://configured.example\n", &[], "litellm_settings.api_base", "https://configured.example")]
#[case::env_openai_base_url("litellm_settings: {}\n", &[("OPENAI_BASE_URL", "https://base.example/v1")], "env.OPENAI_BASE_URL", "https://base.example/v1")]
#[case::env_openai_api_base("litellm_settings: {}\n", &[("OPENAI_API_BASE", "https://legacy.example/v1")], "env.OPENAI_API_BASE", "https://legacy.example/v1")]
#[case::litellm_settings_organization("litellm_settings:\n  organization: org-configured\n", &[], "litellm_settings.organization", "org-configured")]
#[case::env_openai_organization("litellm_settings: {}\n", &[("OPENAI_ORGANIZATION", "org-env")], "env.OPENAI_ORGANIZATION", "org-env")]
#[case::litellm_settings_api_key("litellm_settings:\n  api_key: sk-configured\n", &[], "litellm_settings.api_key", "sk-configured")]
#[case::litellm_settings_openai_key("litellm_settings:\n  openai_key: sk-openai\n", &[], "litellm_settings.openai_key", "sk-openai")]
#[case::env_openai_api_key("litellm_settings: {}\n", &[("OPENAI_API_KEY", "sk-env")], "env.OPENAI_API_KEY", "sk-env")]
#[case::env_azure_api_base("litellm_settings: {}\n", &[("AZURE_API_BASE", "https://azure.example")], "env.AZURE_API_BASE", "https://azure.example")]
#[case::litellm_settings_api_version("litellm_settings:\n  api_version: 2024-02-01\n", &[], "litellm_settings.api_version", "2024-02-01")]
#[case::env_azure_api_version("litellm_settings: {}\n", &[("AZURE_API_VERSION", "2024-06-01")], "env.AZURE_API_VERSION", "2024-06-01")]
#[case::litellm_settings_azure_key("litellm_settings:\n  azure_key: azure-configured\n", &[], "litellm_settings.azure_key", "azure-configured")]
#[case::env_azure_openai_api_key("litellm_settings: {}\n", &[("AZURE_OPENAI_API_KEY", "azure-openai-env")], "env.AZURE_OPENAI_API_KEY", "azure-openai-env")]
#[case::env_azure_api_key("litellm_settings: {}\n", &[("AZURE_API_KEY", "azure-env")], "env.AZURE_API_KEY", "azure-env")]
#[case::env_azure_ad_token("litellm_settings: {}\n", &[("AZURE_AD_TOKEN", "azure-token")], "env.AZURE_AD_TOKEN", "azure-token")]
#[case::litellm_settings_vertex_project("litellm_settings:\n  vertex_project: project-configured\n", &[], "litellm_settings.vertex_project", "project-configured")]
#[case::env_vertexai_project("litellm_settings: {}\n", &[("VERTEXAI_PROJECT", "project-env")], "env.VERTEXAI_PROJECT", "project-env")]
#[case::litellm_settings_vertex_location("litellm_settings:\n  vertex_location: location-configured\n", &[], "litellm_settings.vertex_location", "location-configured")]
#[case::env_vertexai_location("litellm_settings: {}\n", &[("VERTEXAI_LOCATION", "location-env")], "env.VERTEXAI_LOCATION", "location-env")]
#[case::env_vertexai_credentials("litellm_settings: {}\n", &[("VERTEXAI_CREDENTIALS", "{\"type\":\"service_account\"}")], "env.VERTEXAI_CREDENTIALS", "{\"type\":\"service_account\"}")]
fn loads_settings(
    #[case] yaml: &str,
    #[case] environment: &[(&str, &str)],
    #[case] key: &str,
    #[case] expected: &str,
) {
    let settings = settings(yaml, environment);
    assert_eq!(value(&settings, key).as_deref(), Some(expected));
}

#[test]
fn configured_values_win_over_environment_fallbacks() {
    let settings = settings(
        "litellm_settings:\n  api_base: https://configured.example\n  api_key: sk-configured\n  organization: org-configured\n  api_version: 2024-02-01\n  azure_key: azure-configured\n  vertex_project: project-configured\n  vertex_location: location-configured\n",
        &[
            ("OPENAI_BASE_URL", "https://base.example/v1"),
            ("OPENAI_API_BASE", "https://legacy.example/v1"),
            ("OPENAI_API_KEY", "sk-env"),
            ("OPENAI_ORGANIZATION", "org-env"),
            ("AZURE_API_VERSION", "2024-06-01"),
            ("AZURE_API_KEY", "azure-env"),
            ("VERTEXAI_PROJECT", "project-env"),
            ("VERTEXAI_LOCATION", "location-env"),
        ],
    );
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://configured.example")
    );
    assert_eq!(settings.api_version.as_deref(), Some("2024-02-01"));
    assert_eq!(
        settings.vertex_project.as_deref(),
        Some("project-configured")
    );
    assert_eq!(
        settings
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-configured")
    );
    assert_eq!(
        settings
            .openai_api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-env")
    );
}

#[test]
fn os_environ_references_read_the_environment() {
    let settings = settings(
        "litellm_settings:\n  api_base: os.environ/CONFIGURED_BASE\n  api_key: os.environ/CONFIGURED_KEY\n",
        &[
            ("CONFIGURED_BASE", "https://resolved.example"),
            ("CONFIGURED_KEY", "sk-resolved"),
        ],
    );
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://resolved.example")
    );
    assert_eq!(
        settings
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-resolved")
    );
}

#[rstest]
#[case::missing_environment("litellm_settings:\n  api_base: os.environ/GONE\n", "GONE")]
#[case::wrong_scalar(
    "litellm_settings:\n  api_base: [not, text]\n",
    "litellm_settings.api_base"
)]
fn invalid_values_fail_naming_the_field(#[case] yaml: &str, #[case] named: &str) {
    let error = load(yaml, &[]).expect_err("the value cannot be read");
    assert!(
        error.to_string().contains(named),
        "{error} should name {named}"
    );
}

#[test]
fn an_empty_environment_value_stays_empty() {
    let settings = settings("litellm_settings: {}\n", &[("OPENAI_API_KEY", "")]);
    assert_eq!(
        settings
            .openai_api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("")
    );
}
