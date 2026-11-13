#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

type Getter = fn(&Settings) -> Option<String>;

const ALL_KEYS: &str = "\
litellm_settings:
  api_base: https://module.example/v1
  organization: module-org
  api_key: module-key
  openai_key: openai-key
  api_version: 2024-02-01
  azure_key: azure-key
  vertex_project: module-project
  vertex_location: module-location
";

const ALL_ENVIRONMENT: [(&str, &str); 12] = [
    ("OPENAI_BASE_URL", "https://base-url.example/v1"),
    ("OPENAI_API_BASE", "https://api-base.example/v1"),
    ("OPENAI_ORGANIZATION", "environment-org"),
    ("OPENAI_API_KEY", "openai-environment-key"),
    ("AZURE_API_BASE", "https://azure.example"),
    ("AZURE_API_VERSION", "2024-06-01"),
    ("AZURE_OPENAI_API_KEY", "azure-openai-environment-key"),
    ("AZURE_API_KEY", "azure-environment-key"),
    ("AZURE_AD_TOKEN", "azure-ad-token"),
    ("VERTEXAI_PROJECT", "environment-project"),
    ("VERTEXAI_LOCATION", "environment-location"),
    ("VERTEXAI_CREDENTIALS", "environment-credentials"),
];

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .batch_provider_create)
}

fn revealed(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

fn api_base(settings: &Settings) -> Option<String> {
    settings.api_base.clone()
}

fn organization(settings: &Settings) -> Option<String> {
    settings.organization.clone()
}

fn api_key(settings: &Settings) -> Option<String> {
    revealed(settings.api_key.as_ref())
}

fn openai_key(settings: &Settings) -> Option<String> {
    revealed(settings.openai_key.as_ref())
}

fn api_version(settings: &Settings) -> Option<String> {
    settings.api_version.clone()
}

fn azure_key(settings: &Settings) -> Option<String> {
    revealed(settings.azure_key.as_ref())
}

fn vertex_project(settings: &Settings) -> Option<String> {
    settings.vertex_project.clone()
}

fn vertex_location(settings: &Settings) -> Option<String> {
    settings.vertex_location.clone()
}

fn openai_base_url(settings: &Settings) -> Option<String> {
    settings.openai_base_url.clone()
}

fn openai_api_base(settings: &Settings) -> Option<String> {
    settings.openai_api_base.clone()
}

fn openai_organization(settings: &Settings) -> Option<String> {
    settings.openai_organization.clone()
}

fn openai_api_key(settings: &Settings) -> Option<String> {
    revealed(settings.openai_api_key.as_ref())
}

fn azure_api_base(settings: &Settings) -> Option<String> {
    settings.azure_api_base.clone()
}

fn azure_api_version(settings: &Settings) -> Option<String> {
    settings.azure_api_version.clone()
}

fn azure_openai_api_key(settings: &Settings) -> Option<String> {
    revealed(settings.azure_openai_api_key.as_ref())
}

fn azure_api_key(settings: &Settings) -> Option<String> {
    revealed(settings.azure_api_key.as_ref())
}

fn azure_ad_token(settings: &Settings) -> Option<String> {
    revealed(settings.azure_ad_token.as_ref())
}

fn vertexai_project(settings: &Settings) -> Option<String> {
    settings.vertexai_project.clone()
}

fn vertexai_location(settings: &Settings) -> Option<String> {
    settings.vertexai_location.clone()
}

fn vertexai_credentials(settings: &Settings) -> Option<String> {
    revealed(settings.vertexai_credentials.as_ref())
}

#[rstest]
#[case::litellm_settings_api_base(
    "litellm_settings:\n  api_base: https://module.example/v1\n",
    &[],
    api_base,
    Some("https://module.example/v1")
)]
#[case::environment_openai_base_url("", &[("OPENAI_BASE_URL", "https://base-url.example/v1")], openai_base_url, Some("https://base-url.example/v1"))]
#[case::environment_openai_api_base("", &[("OPENAI_API_BASE", "https://api-base.example/v1")], openai_api_base, Some("https://api-base.example/v1"))]
#[case::litellm_settings_organization(
    "litellm_settings:\n  organization: module-org\n",
    &[],
    organization,
    Some("module-org")
)]
#[case::environment_openai_organization("", &[("OPENAI_ORGANIZATION", "environment-org")], openai_organization, Some("environment-org"))]
#[case::litellm_settings_api_key(
    "litellm_settings:\n  api_key: module-key\n",
    &[],
    api_key,
    Some("module-key")
)]
#[case::litellm_settings_openai_key(
    "litellm_settings:\n  openai_key: openai-key\n",
    &[],
    openai_key,
    Some("openai-key")
)]
#[case::environment_openai_api_key("", &[("OPENAI_API_KEY", "openai-environment-key")], openai_api_key, Some("openai-environment-key"))]
#[case::environment_azure_api_base("", &[("AZURE_API_BASE", "https://azure.example")], azure_api_base, Some("https://azure.example"))]
#[case::litellm_settings_api_version(
    "litellm_settings:\n  api_version: 2024-02-01\n",
    &[],
    api_version,
    Some("2024-02-01")
)]
#[case::environment_azure_api_version("", &[("AZURE_API_VERSION", "2024-06-01")], azure_api_version, Some("2024-06-01"))]
#[case::litellm_settings_azure_key(
    "litellm_settings:\n  azure_key: azure-key\n",
    &[],
    azure_key,
    Some("azure-key")
)]
#[case::environment_azure_openai_api_key("", &[("AZURE_OPENAI_API_KEY", "azure-openai-environment-key")], azure_openai_api_key, Some("azure-openai-environment-key"))]
#[case::environment_azure_api_key("", &[("AZURE_API_KEY", "azure-environment-key")], azure_api_key, Some("azure-environment-key"))]
#[case::environment_azure_ad_token("", &[("AZURE_AD_TOKEN", "azure-ad-token")], azure_ad_token, Some("azure-ad-token"))]
#[case::litellm_settings_vertex_project(
    "litellm_settings:\n  vertex_project: module-project\n",
    &[],
    vertex_project,
    Some("module-project")
)]
#[case::environment_vertexai_project("", &[("VERTEXAI_PROJECT", "environment-project")], vertexai_project, Some("environment-project"))]
#[case::litellm_settings_vertex_location(
    "litellm_settings:\n  vertex_location: module-location\n",
    &[],
    vertex_location,
    Some("module-location")
)]
#[case::environment_vertexai_location("", &[("VERTEXAI_LOCATION", "environment-location")], vertexai_location, Some("environment-location"))]
#[case::environment_vertexai_credentials("", &[("VERTEXAI_CREDENTIALS", "environment-credentials")], vertexai_credentials, Some("environment-credentials"))]
fn loads_settings(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] read: Getter,
    #[case] expected: Option<&str>,
) {
    assert_eq!(read(&load("", &[]).unwrap()), None);
    let loaded = load(source, environment).unwrap();
    assert_eq!(read(&loaded).as_deref(), expected);
}

#[test]
fn loads_every_key_at_once() {
    let loaded = load(ALL_KEYS, &ALL_ENVIRONMENT).unwrap();
    assert_eq!(
        loaded.api_base.as_deref(),
        Some("https://module.example/v1")
    );
    assert_eq!(loaded.organization.as_deref(), Some("module-org"));
    assert_eq!(
        revealed(loaded.api_key.as_ref()).as_deref(),
        Some("module-key")
    );
    assert_eq!(
        revealed(loaded.openai_key.as_ref()).as_deref(),
        Some("openai-key")
    );
    assert_eq!(loaded.api_version.as_deref(), Some("2024-02-01"));
    assert_eq!(
        revealed(loaded.azure_key.as_ref()).as_deref(),
        Some("azure-key")
    );
    assert_eq!(loaded.vertex_project.as_deref(), Some("module-project"));
    assert_eq!(loaded.vertex_location.as_deref(), Some("module-location"));
    assert_eq!(
        loaded.openai_base_url.as_deref(),
        Some("https://base-url.example/v1")
    );
    assert_eq!(
        loaded.openai_api_base.as_deref(),
        Some("https://api-base.example/v1")
    );
    assert_eq!(
        loaded.openai_organization.as_deref(),
        Some("environment-org")
    );
    assert_eq!(
        revealed(loaded.openai_api_key.as_ref()).as_deref(),
        Some("openai-environment-key")
    );
    assert_eq!(
        loaded.azure_api_base.as_deref(),
        Some("https://azure.example")
    );
    assert_eq!(loaded.azure_api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(
        revealed(loaded.azure_openai_api_key.as_ref()).as_deref(),
        Some("azure-openai-environment-key")
    );
    assert_eq!(
        revealed(loaded.azure_api_key.as_ref()).as_deref(),
        Some("azure-environment-key")
    );
    assert_eq!(
        revealed(loaded.azure_ad_token.as_ref()).as_deref(),
        Some("azure-ad-token")
    );
    assert_eq!(
        loaded.vertexai_project.as_deref(),
        Some("environment-project")
    );
    assert_eq!(
        loaded.vertexai_location.as_deref(),
        Some("environment-location")
    );
    assert_eq!(
        revealed(loaded.vertexai_credentials.as_ref()).as_deref(),
        Some("environment-credentials")
    );
}

#[test]
fn defaults_are_unset() {
    let loaded = load("litellm_settings: {}\n", &[]).unwrap();
    assert_eq!(
        [
            loaded.api_base,
            loaded.organization,
            loaded.api_version,
            loaded.vertex_project,
            loaded.vertex_location,
            loaded.openai_base_url,
            loaded.openai_api_base,
            loaded.openai_organization,
            loaded.azure_api_base,
            loaded.azure_api_version,
            loaded.vertexai_project,
            loaded.vertexai_location,
        ],
        [
            None, None, None, None, None, None, None, None, None, None, None, None
        ]
    );
    assert!(loaded.api_key.is_none());
    assert!(loaded.openai_key.is_none());
    assert!(loaded.azure_key.is_none());
    assert!(loaded.openai_api_key.is_none());
    assert!(loaded.azure_openai_api_key.is_none());
    assert!(loaded.azure_api_key.is_none());
    assert!(loaded.azure_ad_token.is_none());
    assert!(loaded.vertexai_credentials.is_none());
}

#[test]
fn module_values_and_environment_stay_separate() {
    let loaded = load(
        "litellm_settings:\n  api_base: https://module.example/v1\n  api_key: module-key\n",
        &[
            ("OPENAI_BASE_URL", "https://base-url.example/v1"),
            ("OPENAI_API_KEY", "openai-environment-key"),
        ],
    )
    .unwrap();
    assert_eq!(
        loaded.api_base.as_deref(),
        Some("https://module.example/v1")
    );
    assert_eq!(
        loaded.openai_base_url.as_deref(),
        Some("https://base-url.example/v1")
    );
    assert_eq!(
        revealed(loaded.api_key.as_ref()).as_deref(),
        Some("module-key")
    );
    assert_eq!(
        revealed(loaded.openai_api_key.as_ref()).as_deref(),
        Some("openai-environment-key")
    );
}

#[test]
fn environment_reference_resolves_through_the_lookup() {
    let loaded = load(
        "litellm_settings:\n  api_base: os.environ/MODULE_BASE\n  api_key: os.environ/MODULE_KEY\n",
        &[
            ("MODULE_BASE", "https://resolved.example/v1"),
            ("MODULE_KEY", "resolved-key"),
        ],
    )
    .unwrap();
    assert_eq!(
        loaded.api_base.as_deref(),
        Some("https://resolved.example/v1")
    );
    assert_eq!(
        revealed(loaded.api_key.as_ref()).as_deref(),
        Some("resolved-key")
    );
}

#[test]
fn empty_values_count_as_unset() {
    let loaded = load(
        "litellm_settings:\n  api_base: \"\"\n  api_key: \"\"\n",
        &[
            ("OPENAI_BASE_URL", ""),
            ("OPENAI_API_KEY", ""),
            ("AZURE_API_BASE", ""),
            ("VERTEXAI_CREDENTIALS", ""),
        ],
    )
    .unwrap();
    assert!(loaded.api_base.is_none());
    assert!(loaded.api_key.is_none());
    assert!(loaded.openai_base_url.is_none());
    assert!(loaded.openai_api_key.is_none());
    assert!(loaded.azure_api_base.is_none());
    assert!(loaded.vertexai_credentials.is_none());
}

#[test]
fn missing_environment_reference_is_an_error() {
    let outcome = load(
        "litellm_settings:\n  api_base: os.environ/MODULE_BASE\n",
        &[],
    );
    let Err(ConfigError::MissingEnvironment { variable, path }) = outcome else {
        panic!("a missing reference must name the variable and the setting");
    };
    assert_eq!(variable, "MODULE_BASE");
    assert_eq!(path, "litellm_settings.api_base");
}

#[test]
fn wrong_type_value_is_rejected() {
    let outcome = load("litellm_settings:\n  api_base: [one, two]\n", &[]);
    assert!(matches!(outcome, Err(ConfigError::Yaml { .. })));
}

#[test]
fn empty_reference_value_is_unset() {
    let loaded = load(
        "litellm_settings:\n  api_key: os.environ/MODULE_KEY\n",
        &[("MODULE_KEY", "")],
    )
    .unwrap();
    assert!(loaded.api_key.is_none());
}
