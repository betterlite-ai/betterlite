#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use crate::sections::vertex_embedding_dispatch::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .vertex_embedding_dispatch)
}

#[test]
fn loads_absent_inputs_as_none() {
    let absent = load("", &[]).unwrap();
    assert!(absent.project.is_none());
    assert!(absent.credentials.is_none());
    assert!(absent.api_base.is_none());
}

#[test]
fn loads_every_embedding_alias() {
    let environment = load(
        "",
        &[
            ("VERTEX_PROJECT", "vertex-project"),
            ("VERTEX_CREDENTIALS", "vertex-credentials"),
            ("VERTEXAI_API_BASE", "vertexai-base"),
        ],
    )
    .unwrap();
    assert_eq!(environment.project.as_deref(), Some("vertex-project"));
    assert!(matches!(
        environment.credentials,
        Some(ConfiguredCredentials::Text(_))
    ));
    assert_eq!(environment.api_base.as_deref(), Some("vertexai-base"));
}

#[test]
fn reads_empty_embedding_environment_as_unset() {
    let empty = load(
        "",
        &[
            ("VERTEX_PROJECT", ""),
            ("VERTEX_CREDENTIALS", ""),
            ("VERTEXAI_API_BASE", ""),
            ("VERTEX_API_BASE", ""),
        ],
    )
    .unwrap();
    assert!(empty.project.is_none());
    assert!(empty.credentials.is_none());
    assert!(empty.api_base.is_none());
}

#[test]
fn falls_back_to_the_legacy_api_base() {
    let legacy = load("", &[("VERTEX_API_BASE", "vertex-base")]).unwrap();
    assert_eq!(legacy.api_base.as_deref(), Some("vertex-base"));

    let both = load(
        "",
        &[
            ("VERTEXAI_API_BASE", "vertexai-base"),
            ("VERTEX_API_BASE", "vertex-base"),
        ],
    )
    .unwrap();
    assert_eq!(both.api_base.as_deref(), Some("vertexai-base"));
}

#[test]
fn litellm_settings_api_base_wins_over_the_environment() {
    let configured = load(
        "litellm_settings:\n  api_base: settings-base\n",
        &[
            ("VERTEXAI_API_BASE", "vertexai-base"),
            ("VERTEX_API_BASE", "vertex-base"),
        ],
    )
    .unwrap();
    assert_eq!(configured.api_base.as_deref(), Some("settings-base"));
}

#[test]
fn resolves_an_environment_reference_in_the_module_api_base() {
    let referenced = load(
        "litellm_settings:\n  api_base: os.environ/VERTEX_MODULE_BASE\n",
        &[("VERTEX_MODULE_BASE", "referenced-base")],
    )
    .unwrap();
    assert_eq!(referenced.api_base.as_deref(), Some("referenced-base"));
}

#[test]
fn parses_vertex_credentials_as_a_credential_source() {
    let source = r#"{"type": "service_account", "project_id": "project", "private_key": "key", "client_email": "svc@example.com"}"#;
    let parsed = load("", &[("VERTEX_CREDENTIALS", source)]).unwrap();
    assert!(matches!(
        parsed.credentials,
        Some(ConfiguredCredentials::Source(_))
    ));
}
