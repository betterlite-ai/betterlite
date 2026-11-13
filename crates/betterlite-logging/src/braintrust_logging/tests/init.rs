use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::support::{KEY, settings};
use crate::braintrust_logging::settings::{Settings, SettingsError};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn init_exposes_the_key_and_the_default_base() {
    let resolved = Settings::resolve(
        Some(SecretString::from("arg-key")),
        None,
        Vec::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "arg-key");
    assert_eq!(resolved.api_base, "https://api.braintrustdata.com/v1");
}

#[test]
fn init_reads_the_key_from_the_environment() {
    let variables = environment(&[("BRAINTRUST_API_KEY", "env-key")]);
    let resolved = Settings::resolve(None, None, Vec::new(), &variables).unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "env-key");
    assert_eq!(resolved.api_base, "https://api.braintrustdata.com/v1");
}

#[test]
fn init_prefers_the_argument_over_the_environment() {
    let variables = environment(&[
        ("BRAINTRUST_API_KEY", "env-key"),
        ("BRAINTRUST_API_BASE", "https://env.example/v1"),
    ]);
    let resolved = Settings::resolve(
        Some(SecretString::from("arg-key")),
        Some("https://arg.example/v1".to_owned()),
        vec!["user_id".to_owned()],
        &variables,
    )
    .unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "arg-key");
    assert_eq!(resolved.api_base, "https://arg.example/v1");
    assert_eq!(resolved.default_tags, ["user_id"]);
}

#[test]
fn init_treats_empty_environment_values_as_unset() {
    let variables = environment(&[("BRAINTRUST_API_KEY", ""), ("BRAINTRUST_API_BASE", "")]);
    let missing = Settings::resolve(None, None, Vec::new(), &variables);
    assert_eq!(missing.unwrap_err(), SettingsError::MissingApiKey);
    let resolved = Settings::resolve(
        Some(SecretString::from("arg-key")),
        None,
        Vec::new(),
        &variables,
    )
    .unwrap();
    assert_eq!(resolved.api_base, "https://api.braintrustdata.com/v1");
}

#[test]
fn init_debug_output_hides_the_key() {
    let settings = settings(&[]);
    assert!(!format!("{settings:?}").contains(KEY));
}
