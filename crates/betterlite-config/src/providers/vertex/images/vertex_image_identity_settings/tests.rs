#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

use super::{ImageEnvironmentTier, ImageModuleTier, ImageSecretTier};

fn lookup(env: &[(&str, &str)]) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn identity(
    source: &str,
    env: &[(&str, &str)],
) -> (ImageEnvironmentTier, ImageModuleTier, ImageSecretTier) {
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
    let (environment, module, secrets) = identity(
        CONFIGURED_MODULE,
        &[
            ("VERTEXAI_PROJECT", "env-project"),
            ("VERTEXAI_LOCATION", "us-central1"),
        ],
    );
    assert_eq!(
        module,
        (
            Some("config-project".to_owned()),
            Some("asia-east1".to_owned())
        )
    );
    assert_eq!(environment.0.as_deref(), Some("env-project"));
    assert_eq!(environment.1.as_deref(), Some("us-central1"));
    assert_eq!(secrets.0.as_deref(), Some("env-project"));
}

#[test]
fn the_two_location_spellings_are_kept_apart() {
    let (environment, module, secrets) = identity(
        "",
        &[
            ("VERTEXAI_LOCATION", "us-central1"),
            ("VERTEX_LOCATION", "legacy-location"),
        ],
    );
    assert_eq!(module, (None, None));
    assert_eq!(environment.1.as_deref(), Some("us-central1"));
    assert_eq!(environment.2.as_deref(), Some("legacy-location"));
    assert_eq!(secrets.1.as_deref(), Some("us-central1"));
    assert_eq!(secrets.2.as_deref(), Some("legacy-location"));
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let (environment, _module, secrets) =
        identity("", &[("VERTEXAI_PROJECT", ""), ("VERTEXAI_LOCATION", "")]);
    assert_eq!(environment.0, None);
    assert_eq!(environment.1, None);
    assert_eq!(secrets.0, None);
}

#[test]
fn the_environment_credentials_and_the_application_defaults_load_as_configured_credentials() {
    let (environment, _module, secrets) = identity(
        "",
        &[
            ("VERTEXAI_CREDENTIALS", "/keys/service-account.json"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/keys/application.json"),
        ],
    );
    assert_eq!(
        environment
            .3
            .as_ref()
            .and_then(|credentials| credentials.as_text()),
        Some("/keys/service-account.json")
    );
    assert_eq!(
        secrets
            .3
            .as_ref()
            .and_then(|credentials| credentials.as_text()),
        Some("/keys/service-account.json")
    );
    assert_eq!(
        environment
            .4
            .as_ref()
            .and_then(|credentials| credentials.as_text()),
        Some("/keys/application.json")
    );
}

#[test]
fn a_credential_document_loads_as_a_source_rather_than_text() {
    let document =
        r#"{"type":"service_account","private_key":"key","client_email":"sa@example.com"}"#;
    let (environment, _module, _secrets) = identity("", &[("VERTEXAI_CREDENTIALS", document)]);
    let credentials = environment.3.expect("the document parses");
    assert_eq!(credentials.as_text(), None);
}

#[test]
fn an_os_environ_reference_in_the_module_setting_is_resolved() {
    let (_environment, module, _secrets) = identity(
        "litellm_settings:\n  vertex_project: os.environ/PROJECT_FROM_ENV\n",
        &[("PROJECT_FROM_ENV", "resolved-project")],
    );
    assert_eq!(module.0.as_deref(), Some("resolved-project"));
}

#[test]
fn an_empty_module_setting_counts_as_unset() {
    let (_environment, module, _secrets) =
        identity("litellm_settings:\n  vertex_project: \"\"\n", &[]);
    assert_eq!(module, (None, None));
}

#[test]
fn a_credentials_module_setting_is_an_unknown_key() {
    let error = ConfigLoader {
        lookup: lookup(&[]),
    }
    .load("litellm_settings:\n  vertex_credentials: /keys/service-account.json\n")
    .unwrap_err();
    assert!(
        matches!(
            &error,
            ConfigError::Unsupported { paths }
                if *paths == ["litellm_settings.vertex_credentials"]
        ),
        "{error}"
    );
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
