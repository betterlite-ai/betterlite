#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Inner, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(*ConfigLoader { lookup }
        .load(source)?
        .sections
        .data_cloud_credentials
        .inner)
}

#[test]
fn data_cloud_credentials_defaults_to_no_key() {
    let settings = load("", &[]).expect("an empty config loads");
    assert!(settings.databricks_key.is_none());
}

#[test]
fn data_cloud_credentials_loads_the_databricks_key() {
    let source = "litellm_settings:\n  databricks_key: db-secret\n";
    let settings = load(source, &[]).expect("the key loads");
    assert_eq!(
        settings
            .databricks_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("db-secret")
    );
}

#[test]
fn data_cloud_credentials_resolves_an_environment_reference() {
    let source = "litellm_settings:\n  databricks_key: os.environ/DATABRICKS_KEY_REFERENCE\n";
    let settings =
        load(source, &[("DATABRICKS_KEY_REFERENCE", "from-env")]).expect("the reference resolves");
    assert_eq!(
        settings
            .databricks_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("from-env")
    );
}

#[test]
fn data_cloud_credentials_reports_a_missing_environment_reference() {
    let source = "litellm_settings:\n  databricks_key: os.environ/DATABRICKS_KEY_MISSING\n";
    let error = load(source, &[]).expect_err("an unresolved reference fails the load");
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("a missing reference is a missing environment error");
    };
    assert_eq!(variable, "DATABRICKS_KEY_MISSING");
    assert_eq!(path, "litellm_settings.databricks_key");
}

#[test]
fn data_cloud_credentials_keeps_an_explicit_empty_key() {
    let source = "litellm_settings:\n  databricks_key: \"\"\n";
    let settings = load(source, &[]).expect("an empty key loads");
    assert_eq!(
        settings
            .databricks_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("")
    );
}

#[test]
fn data_cloud_credentials_rejects_an_unrelated_key() {
    let source = "litellm_settings:\n  data_cloud_credentials_unrelated_key: 1\n";
    let error = load(source, &[]).expect_err("an unrelated key is unsupported");
    let ConfigError::Unsupported { paths } = error else {
        panic!("an unrelated key is reported as unsupported");
    };
    assert!(
        paths
            .iter()
            .any(|path| path.contains("data_cloud_credentials_unrelated_key"))
    );
}
