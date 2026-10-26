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
    let config = ConfigLoader { lookup: environment }
        .load("litellm_settings: {}\n")
        .expect("settings load");
    let key = config
        .sections
        .resource_sdk_settings
        .openai_api_key
        .expect("empty key is present");
    assert!(key.expose_secret().is_empty());
}
