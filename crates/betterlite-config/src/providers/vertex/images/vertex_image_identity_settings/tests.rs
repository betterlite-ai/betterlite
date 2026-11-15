#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

use super::VertexImageIdentitySettings;

fn lookup(env: &[(&str, &str)]) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn identity(source: &str, env: &[(&str, &str)]) -> VertexImageIdentitySettings {
    let lookup = lookup(env);
    let config = ConfigLoader {
        lookup: lookup.clone(),
    }
    .load(source)
    .expect("the config loads");
    config
        .sections
        .vertex_image_identity_settings(&lookup)
        .expect("the image identity resolves")
}

const CONFIGURED_MODULE: &str =
    "litellm_settings:\n  vertex_project: config-project\n  vertex_location: asia-east1\n";

#[test]
fn the_module_setting_loads_beside_the_environment_it_beats() {
    let settings = identity(
        CONFIGURED_MODULE,
        &[
            ("VERTEXAI_PROJECT", "env-project"),
            ("VERTEXAI_LOCATION", "us-central1"),
        ],
    );
    assert_eq!(settings.module.project.as_deref(), Some("config-project"));
    assert_eq!(settings.module.location.as_deref(), Some("asia-east1"));
    assert_eq!(settings.environment.project.as_deref(), Some("env-project"));
    assert_eq!(
        settings.environment.location.as_deref(),
        Some("us-central1")
    );
    assert_eq!(settings.secrets.project.as_deref(), Some("env-project"));
}

#[test]
fn the_two_location_spellings_are_kept_apart() {
    let settings = identity(
        "",
        &[
            ("VERTEXAI_LOCATION", "us-central1"),
            ("VERTEX_LOCATION", "legacy-location"),
        ],
    );
    assert_eq!(settings.module.project, None);
    assert_eq!(settings.module.location, None);
    assert_eq!(
        settings.environment.location.as_deref(),
        Some("us-central1")
    );
    assert_eq!(
        settings.environment.legacy_location.as_deref(),
        Some("legacy-location")
    );
    assert_eq!(settings.secrets.location.as_deref(), Some("us-central1"));
    assert_eq!(
        settings.secrets.legacy_location.as_deref(),
        Some("legacy-location")
    );
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let settings = identity("", &[("VERTEXAI_PROJECT", ""), ("VERTEXAI_LOCATION", "")]);
    assert_eq!(settings.environment.project, None);
    assert_eq!(settings.environment.location, None);
    assert_eq!(settings.secrets.project, None);
}

#[test]
fn the_environment_credentials_and_the_application_defaults_load_as_configured_credentials() {
    let settings = identity(
        "",
        &[
            ("VERTEXAI_CREDENTIALS", "/keys/service-account.json"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/keys/application.json"),
        ],
    );
    assert_eq!(
        settings
            .environment
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/keys/service-account.json")
    );
    assert_eq!(
        settings
            .secrets
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/keys/service-account.json")
    );
    assert_eq!(
        settings
            .environment
            .application_credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/keys/application.json")
    );
}

#[test]
fn a_credential_document_loads_as_a_source_rather_than_text() {
    let document =
        r#"{"type":"service_account","private_key":"key","client_email":"sa@example.com"}"#;
    let settings = identity("", &[("VERTEXAI_CREDENTIALS", document)]);
    let credentials = settings
        .environment
        .credentials
        .expect("the document parses");
    assert_eq!(credentials.as_text(), None);
}

#[test]
fn an_os_environ_reference_in_the_module_setting_is_resolved() {
    let settings = identity(
        "litellm_settings:\n  vertex_project: os.environ/PROJECT_FROM_ENV\n",
        &[("PROJECT_FROM_ENV", "resolved-project")],
    );
    assert_eq!(settings.module.project.as_deref(), Some("resolved-project"));
}

#[test]
fn an_empty_module_setting_counts_as_unset() {
    let settings = identity(
        "litellm_settings:\n  vertex_project: \"\"\n  vertex_location: \"\"\n",
        &[],
    );
    assert_eq!(settings.module.project, None);
    assert_eq!(settings.module.location, None);
}

#[test]
fn an_application_default_that_is_a_broken_document_names_the_variable() {
    let lookup = lookup(&[(
        "GOOGLE_APPLICATION_CREDENTIALS",
        r#"{"type":"service_account"}"#,
    )]);
    let config = ConfigLoader {
        lookup: lookup.clone(),
    }
    .load("")
    .expect("the empty config loads");
    let error = config
        .sections
        .vertex_image_identity_settings(&lookup)
        .expect_err("the credential document is incomplete");
    assert_eq!(error.category(), "invalid_scalar");
}

#[test]
fn the_module_credentials_load_apart_from_the_environment_credentials() {
    let settings = identity(
        "litellm_settings:\n  vertex_credentials: /keys/module.json\n",
        &[("VERTEXAI_CREDENTIALS", "/keys/environment.json")],
    );
    assert_eq!(
        settings
            .module
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/keys/module.json")
    );
    assert_eq!(
        settings
            .environment
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/keys/environment.json")
    );
}

#[test]
fn the_module_credentials_are_absent_without_the_key_and_ignore_the_environment() {
    let settings = identity("", &[("VERTEXAI_CREDENTIALS", "/keys/environment.json")]);
    assert!(settings.module.credentials.is_none());
}

#[test]
fn an_os_environ_reference_in_the_module_credentials_is_resolved() {
    let settings = identity(
        "litellm_settings:\n  vertex_credentials: os.environ/MODULE_KEYS\n",
        &[("MODULE_KEYS", "/keys/from-env.json")],
    );
    assert_eq!(
        settings
            .module
            .credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/keys/from-env.json")
    );
}

#[test]
fn a_module_credential_document_loads_as_a_source_and_a_broken_one_names_the_key() {
    let document =
        r#"{"type":"service_account","private_key":"key","client_email":"sa@example.com"}"#;
    let settings = identity(
        &format!("litellm_settings:\n  vertex_credentials: '{document}'\n"),
        &[],
    );
    let credentials = settings.module.credentials.expect("the document parses");
    assert_eq!(credentials.as_text(), None);
    let broken = ConfigLoader {
        lookup: lookup(&[]),
    }
    .load("litellm_settings:\n  vertex_credentials: '{\"type\":\"service_account\"}'\n")
    .expect_err("the credential document is incomplete");
    assert_eq!(broken.category(), "invalid_scalar");
}
