#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::deepseek_identity::{DEFAULT_API_BASE, Settings};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .deepseek_identity
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

#[rstest]
#[case::both_keys(&[("DEEPSEEK_API_BASE", "https://env.example"), ("DEEPSEEK_API_KEY", "env-key")], Some("https://env.example"), Some("env-key"))]
#[case::base_only(&[("DEEPSEEK_API_BASE", "https://env.example")], Some("https://env.example"), None)]
#[case::key_only(&[("DEEPSEEK_API_KEY", "env-key")], None, Some("env-key"))]
#[case::nothing(&[], None, None)]
#[case::empty_values(&[("DEEPSEEK_API_BASE", ""), ("DEEPSEEK_API_KEY", "")], None, None)]
fn deepseek_environment_keys_load(
    #[case] environment: &[(&str, &str)],
    #[case] expected_base: Option<&str>,
    #[case] expected_key: Option<&str>,
) {
    let loaded = settings(environment);
    assert_eq!(loaded.api_base.as_deref(), expected_base);
    let loaded_key = loaded
        .api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    assert_eq!(loaded_key.as_deref(), expected_key);
}

#[rstest]
#[case::explicit_wins(&[("DEEPSEEK_API_BASE", "https://env.example"), ("DEEPSEEK_API_KEY", "env-key")], Some("https://explicit.example"), Some("explicit-key"), "https://explicit.example", Some("explicit-key"))]
#[case::absent_takes_environment(&[("DEEPSEEK_API_BASE", "https://env.example"), ("DEEPSEEK_API_KEY", "env-key")], None, None, "https://env.example", Some("env-key"))]
#[case::empty_falls_through(&[("DEEPSEEK_API_BASE", "https://env.example"), ("DEEPSEEK_API_KEY", "env-key")], Some(""), Some(""), "https://env.example", Some("env-key"))]
#[case::nothing_anywhere(&[], None, None, DEFAULT_API_BASE, None)]
#[case::empty_without_environment(&[], Some(""), Some(""), DEFAULT_API_BASE, None)]
#[case::base_without_key(&[("DEEPSEEK_API_BASE", "https://env.example")], None, None, "https://env.example", None)]
#[case::key_without_base(&[("DEEPSEEK_API_KEY", "env-key")], None, None, DEFAULT_API_BASE, Some("env-key"))]
fn deepseek_identity_resolution_follows_the_pinned_fallbacks(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected_base: &str,
    #[case] expected_key: Option<&str>,
) {
    let supplied_key = explicit_key.map(secret);
    let (base, key) = settings(environment).resolve(explicit_base, supplied_key.as_ref());
    assert_eq!(base, expected_base);
    let resolved_key = key.as_ref().map(|secret| secret.expose_secret().to_owned());
    assert_eq!(resolved_key.as_deref(), expected_key);
}

#[test]
fn deepseek_key_has_no_default() {
    let (base, key) = settings(&[]).resolve(None, None);
    assert_eq!(base, DEFAULT_API_BASE);
    assert!(key.is_none());
}
