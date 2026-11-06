#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::pangea::config::{DEFAULT_API_BASE, PangeaGuardrailConfigModel};
use crate::pangea::error::PangeaGuardrailError;
use crate::pangea::hooks::PangeaGuardrail;
use crate::pangea::tests::support::GUARDRAIL_NAME;

fn resolve(
    lookup: &BTreeMap<String, String>,
    api_key: Option<&str>,
    api_base: Option<&str>,
) -> Result<PangeaGuardrail, PangeaGuardrailError> {
    let config = PangeaGuardrailConfigModel {
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        api_base: api_base.map(str::to_owned),
        optional_params: None,
    };
    PangeaGuardrail::new(
        Some(GUARDRAIL_NAME.to_owned()),
        config,
        lookup,
        reqwest::Client::default(),
    )
}

#[test]
fn environment_config_api_key_beats_the_api_key_variable() {
    let lookup = BTreeMap::from([("PANGEA_API_KEY".to_owned(), "env-key".to_owned())]);
    let guardrail = resolve(&lookup, Some("config-key"), None).expect("the guardrail resolves");
    assert_eq!(guardrail.api_key.expose_secret(), "config-key");
}

#[test]
fn environment_reads_the_api_key_from_pangea_api_key() {
    let lookup = BTreeMap::from([("PANGEA_API_KEY".to_owned(), "env-key".to_owned())]);
    let guardrail = resolve(&lookup, None, None).expect("the guardrail resolves");
    assert_eq!(guardrail.api_key.expose_secret(), "env-key");
}

#[test]
fn environment_config_api_base_beats_the_api_base_variable() {
    let lookup = BTreeMap::from([("PANGEA_API_BASE".to_owned(), "https://env.test".to_owned())]);
    let guardrail = resolve(&lookup, Some("key"), Some("https://config.test")).expect("resolves");
    assert_eq!(guardrail.api_base, "https://config.test");
}

#[test]
fn environment_reads_the_api_base_from_pangea_api_base() {
    let lookup = BTreeMap::from([("PANGEA_API_BASE".to_owned(), "https://env.test".to_owned())]);
    let guardrail = resolve(&lookup, Some("key"), None).expect("the guardrail resolves");
    assert_eq!(guardrail.api_base, "https://env.test");
}

#[test]
fn environment_defaults_the_api_base_to_the_pangea_guard_host() {
    let guardrail = resolve(&BTreeMap::new(), Some("key"), None).expect("the guardrail resolves");
    assert_eq!(guardrail.api_base, DEFAULT_API_BASE);
}

#[test]
fn environment_empty_api_key_variable_counts_as_unset() {
    let lookup = BTreeMap::from([("PANGEA_API_KEY".to_owned(), String::new())]);
    let error = resolve(&lookup, None, None).expect_err("an empty key is not a key");
    assert!(matches!(error, PangeaGuardrailError::MissingSecrets));
}

#[test]
fn environment_empty_api_base_variable_counts_as_unset() {
    let lookup = BTreeMap::from([("PANGEA_API_BASE".to_owned(), String::new())]);
    let guardrail = resolve(&lookup, Some("key"), None).expect("the guardrail resolves");
    assert_eq!(guardrail.api_base, DEFAULT_API_BASE);
}

#[test]
fn environment_recipes_are_not_read_from_environment_variables() {
    let lookup = BTreeMap::from([
        ("PANGEA_API_KEY".to_owned(), "env-key".to_owned()),
        ("PANGEA_INPUT_RECIPE".to_owned(), "env-input".to_owned()),
        ("PANGEA_OUTPUT_RECIPE".to_owned(), "env-output".to_owned()),
    ]);
    let guardrail = resolve(&lookup, None, None).expect("the guardrail resolves");
    assert_eq!(guardrail.input_recipe, None);
    assert_eq!(guardrail.output_recipe, None);
}
