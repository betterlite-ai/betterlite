#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .model_hub_credentials)
}

fn text(value: Option<&secrecy::SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn model_hub_credentials_defaults_to_no_keys() {
    let settings = load("", &[]).expect("an empty config loads");
    assert!(settings.huggingface_key.is_none());
    assert!(settings.replicate_key.is_none());
    assert!(settings.predibase_key.is_none());
    assert!(settings.predibase_tenant_id.is_none());
}

#[test]
fn model_hub_credentials_loads_each_litellm_setting() {
    let source = "litellm_settings:\n  huggingface_key: hf-secret\n  replicate_key: replicate-secret\n  predibase_key: predibase-secret\n  predibase_tenant_id: tenant-1\n";
    let settings = load(source, &[]).expect("the four keys load");
    assert_eq!(text(settings.huggingface_key.as_ref()).as_deref(), Some("hf-secret"));
    assert_eq!(text(settings.replicate_key.as_ref()).as_deref(), Some("replicate-secret"));
    assert_eq!(text(settings.predibase_key.as_ref()).as_deref(), Some("predibase-secret"));
    assert_eq!(text(settings.predibase_tenant_id.as_ref()).as_deref(), Some("tenant-1"));
}

#[test]
fn model_hub_credentials_resolves_an_environment_reference() {
    let source = "litellm_settings:\n  huggingface_key: os.environ/HF_KEY_REFERENCE\n";
    let settings = load(source, &[("HF_KEY_REFERENCE", "from-env")]).expect("the reference resolves");
    assert_eq!(text(settings.huggingface_key.as_ref()).as_deref(), Some("from-env"));
}

#[test]
fn model_hub_credentials_reports_a_missing_environment_reference() {
    let source = "litellm_settings:\n  predibase_tenant_id: os.environ/PREDIBASE_TENANT_REFERENCE\n";
    let error = load(source, &[]).expect_err("an unresolved reference fails the load");
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("a missing reference is a missing environment error");
    };
    assert_eq!(variable, "PREDIBASE_TENANT_REFERENCE");
    assert_eq!(path, "litellm_settings.predibase_tenant_id");
}

#[test]
fn model_hub_credentials_keeps_an_explicit_empty_key() {
    let source = "litellm_settings:\n  replicate_key: \"\"\n";
    let settings = load(source, &[]).expect("an empty key loads");
    assert_eq!(text(settings.replicate_key.as_ref()).as_deref(), Some(""));
}

#[test]
fn model_hub_credentials_rejects_an_unrelated_key() {
    let source = "litellm_settings:\n  model_hub_credentials_unrelated_key: 1\n";
    let error = load(source, &[]).expect_err("an unrelated key is unsupported");
    let ConfigError::Unsupported { paths } = error else {
        panic!("an unrelated key is reported as unsupported");
    };
    assert!(paths.iter().any(|path| path.contains("model_hub_credentials_unrelated_key")));
}
