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
        .enterprise_ml_credentials
        .inner)
}

#[test]
fn enterprise_ml_credentials_defaults_to_no_service_key() {
    let settings = load("", &[]).expect("an empty config loads");
    assert!(settings.sap_service_key.is_none());
}

#[test]
fn enterprise_ml_credentials_loads_the_sap_service_key() {
    let source = "litellm_settings:\n  sap_service_key: sap-secret\n";
    let settings = load(source, &[]).expect("the key loads");
    assert_eq!(
        settings
            .sap_service_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sap-secret")
    );
}

#[test]
fn enterprise_ml_credentials_resolves_an_environment_reference() {
    let source = "litellm_settings:\n  sap_service_key: os.environ/SAP_SERVICE_KEY_REFERENCE\n";
    let settings =
        load(source, &[("SAP_SERVICE_KEY_REFERENCE", "from-env")]).expect("the reference resolves");
    assert_eq!(
        settings
            .sap_service_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("from-env")
    );
}

#[test]
fn enterprise_ml_credentials_reports_a_missing_environment_reference() {
    let source = "litellm_settings:\n  sap_service_key: os.environ/SAP_SERVICE_KEY_MISSING\n";
    let error = load(source, &[]).expect_err("an unresolved reference fails the load");
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("a missing reference is a missing environment error");
    };
    assert_eq!(variable, "SAP_SERVICE_KEY_MISSING");
    assert_eq!(path, "litellm_settings.sap_service_key");
}

#[test]
fn enterprise_ml_credentials_keeps_an_explicit_empty_service_key() {
    let source = "litellm_settings:\n  sap_service_key: \"\"\n";
    let settings = load(source, &[]).expect("an empty key loads");
    assert_eq!(
        settings
            .sap_service_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("")
    );
}

#[test]
fn enterprise_ml_credentials_rejects_an_unrelated_key() {
    let source = "litellm_settings:\n  enterprise_ml_credentials_unrelated_key: 1\n";
    let error = load(source, &[]).expect_err("an unrelated key is unsupported");
    let ConfigError::Unsupported { paths } = error else {
        panic!("an unrelated key is reported as unsupported");
    };
    assert!(
        paths
            .iter()
            .any(|path| path.contains("enterprise_ml_credentials_unrelated_key"))
    );
}
