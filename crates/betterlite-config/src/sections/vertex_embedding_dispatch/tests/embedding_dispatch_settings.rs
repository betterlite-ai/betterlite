#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

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
        environment.credentials.as_deref(),
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
        parsed.credentials.as_deref(),
        Some(ConfiguredCredentials::Source(_))
    ));
}

#[test]
fn keeps_only_the_embedding_options_of_each_deployment() {
    let source = "model_list:\n  - model_name: embed\n    litellm_params:\n      model: vertex_ai/text-embedding-004\n      vertex_project: os.environ/PIN_PROJECT\n      vertex_ai_location: us-east1\n      api_key: sk-private\n  - model_name: bare\n    litellm_params:\n      model: vertex_ai/other\n";
    let loaded = load(source, &[("PIN_PROJECT", "pinned-project")]).unwrap();
    assert_eq!(loaded.deployments.len(), 2);
    let first = &loaded.deployments[0].0;
    assert_eq!(
        first.get("vertex_project"),
        Some(&Json::Text("pinned-project".to_owned()))
    );
    assert_eq!(
        first.get("vertex_ai_location"),
        Some(&Json::Text("us-east1".to_owned()))
    );
    assert_eq!(first.len(), 2);
    assert!(loaded.deployments[1].0.is_empty());
}

#[test]
fn names_a_missing_environment_variable_in_a_deployment_pin() {
    let source = "model_list:\n  - model_name: embed\n    litellm_params:\n      model: vertex_ai/x\n      vertex_credentials: os.environ/NOT_SET\n";
    let error = load(source, &[]).unwrap_err();
    assert!(error.to_string().contains("NOT_SET"));
}

#[test]
fn debug_output_of_a_deployment_pin_hides_its_values() {
    let source = "model_list:\n  - model_name: embed\n    litellm_params:\n      model: vertex_ai/x\n      vertex_credentials: very-secret-document\n";
    let loaded = load(source, &[]).unwrap();
    let shown = format!("{:?}", loaded.deployments[0]);
    assert!(shown.contains("vertex_credentials"));
    assert!(!shown.contains("very-secret-document"));
}
