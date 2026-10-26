#![cfg(test)]

use super::super::super::loader::ConfigLoader;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;

#[test]
fn loads_resource_sdk_settings() {
    let mut environment = BTreeMap::new();
    environment.insert("OPENAI_API_KEY".to_owned(), "env-key".to_owned());
    environment.insert("AZURE_AD_TOKEN".to_owned(), "azure-token".to_owned());
    let config = ConfigLoader { lookup: environment }
        .load(
            "litellm_settings:\n  api_base: https://global.example\n  api_key: os.environ/OPENAI_API_KEY\n  vertex_project: project\n",
        )
        .expect("settings load");
    let settings = config.sections.resource_sdk_settings;
    assert_eq!(settings.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(settings.vertex_project.as_deref(), Some("project"));
    assert_eq!(
        settings.api_key.expect("configured key").expose_secret(),
        "env-key"
    );
    assert_eq!(
        settings
            .azure_ad_token
            .expect("azure token")
            .expose_secret(),
        "azure-token"
    );
}

#[test]
fn preserves_empty_environment_values() {
    let mut environment = BTreeMap::new();
    environment.insert("OPENAI_API_KEY".to_owned(), String::new());
    let config = ConfigLoader {
        lookup: environment,
    }
    .load("litellm_settings: {}\n")
    .expect("settings load");
    let key = config
        .sections
        .resource_sdk_settings
        .openai_api_key
        .expect("empty key is present");
    assert!(key.expose_secret().is_empty());
}

#[test]
fn loads_all_resource_sdk_environment_overrides() {
    let settings = environment_settings();
    assert_eq!(settings.openai_base_url.as_deref(), Some("openai-base"));
    assert_eq!(settings.openai_api_base.as_deref(), Some("legacy-base"));
    assert_eq!(settings.openai_organization.as_deref(), Some("openai-org"));
    assert_eq!(settings.azure_api_base.as_deref(), Some("azure-base"));
    assert_eq!(settings.azure_api_version.as_deref(), Some("azure-version"));
    assert_eq!(settings.vertexai_project.as_deref(), Some("vertex-project"));
    assert_eq!(
        settings.vertexai_location.as_deref(),
        Some("vertex-location")
    );
}

#[test]
fn loads_all_resource_sdk_environment_secrets() {
    let settings = environment_settings();
    assert_eq!(
        settings.openai_api_key.expect("OpenAI key").expose_secret(),
        "openai-key"
    );
    assert_eq!(
        settings
            .azure_openai_api_key
            .expect("Azure OpenAI key")
            .expose_secret(),
        "azure-openai-key"
    );
    assert_eq!(
        settings.azure_api_key.expect("Azure key").expose_secret(),
        "azure-key"
    );
    assert_eq!(
        settings
            .vertexai_credentials
            .expect("Vertex credentials")
            .expose_secret(),
        "vertex-credentials"
    );
}

fn environment_settings() -> super::Settings {
    let environment = BTreeMap::from([
        ("OPENAI_BASE_URL".to_owned(), "openai-base".to_owned()),
        ("OPENAI_API_BASE".to_owned(), "legacy-base".to_owned()),
        ("OPENAI_API_KEY".to_owned(), "openai-key".to_owned()),
        ("OPENAI_ORGANIZATION".to_owned(), "openai-org".to_owned()),
        ("AZURE_API_BASE".to_owned(), "azure-base".to_owned()),
        ("AZURE_API_VERSION".to_owned(), "azure-version".to_owned()),
        (
            "AZURE_OPENAI_API_KEY".to_owned(),
            "azure-openai-key".to_owned(),
        ),
        ("AZURE_API_KEY".to_owned(), "azure-key".to_owned()),
        ("VERTEXAI_PROJECT".to_owned(), "vertex-project".to_owned()),
        ("VERTEXAI_LOCATION".to_owned(), "vertex-location".to_owned()),
        (
            "VERTEXAI_CREDENTIALS".to_owned(),
            "vertex-credentials".to_owned(),
        ),
    ]);
    ConfigLoader {
        lookup: environment,
    }
    .load("litellm_settings: {}\n")
    .expect("settings load")
    .sections
    .resource_sdk_settings
}

#[test]
fn loads_all_resource_sdk_settings_from_yaml() {
    let environment = BTreeMap::from([("CONFIGURED_SECRET".to_owned(), "secret".to_owned())]);
    let config = ConfigLoader { lookup: environment }
        .load(
            "litellm_settings:\n  api_base: base\n  api_version: version\n  api_key: os.environ/CONFIGURED_SECRET\n  openai_key: openai\n  azure_key: azure\n  organization: org\n  vertex_project: project\n  vertex_location: location\n",
        )
        .expect("settings load");
    let settings = config.sections.resource_sdk_settings;
    assert_eq!(settings.api_base.as_deref(), Some("base"));
    assert_eq!(settings.api_version.as_deref(), Some("version"));
    assert_eq!(settings.organization.as_deref(), Some("org"));
    assert_eq!(settings.vertex_project.as_deref(), Some("project"));
    assert_eq!(settings.vertex_location.as_deref(), Some("location"));
    assert_eq!(settings.api_key.expect("API key").expose_secret(), "secret");
    assert_eq!(
        settings.openai_key.expect("OpenAI key").expose_secret(),
        "openai"
    );
    assert_eq!(
        settings.azure_key.expect("Azure key").expose_secret(),
        "azure"
    );
}
