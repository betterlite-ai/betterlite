#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::openai_like_settings::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .openai_like_settings
}

#[rstest]
#[case::both(&[("OPENAI_LIKE_API_BASE", "http://vllm:8000/v1"), ("OPENAI_LIKE_API_KEY", "sk-env")], Some("http://vllm:8000/v1"), Some("sk-env"))]
#[case::base_only(&[("OPENAI_LIKE_API_BASE", "http://vllm:8000/v1")], Some("http://vllm:8000/v1"), None)]
#[case::key_only(&[("OPENAI_LIKE_API_KEY", "sk-env")], None, Some("sk-env"))]
#[case::empty_values_count_as_unset(&[("OPENAI_LIKE_API_BASE", ""), ("OPENAI_LIKE_API_KEY", "")], None, None)]
#[case::absent(&[], None, None)]
fn loads_both_environment_keys(
    #[case] environment: &[(&str, &str)],
    #[case] base: Option<&str>,
    #[case] key: Option<&str>,
) {
    let loaded = settings(environment);
    assert_eq!(loaded.api_base.as_deref(), base);
    let loaded_key = loaded
        .api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    assert_eq!(loaded_key.as_deref(), key);
}

const ENVIRONMENT: &[(&str, &str)] = &[
    ("OPENAI_LIKE_API_BASE", "http://env/v1"),
    ("OPENAI_LIKE_API_KEY", "sk-env"),
];

#[rstest]
#[case::explicit_wins(
    ENVIRONMENT,
    Some("http://explicit/v1"),
    Some("sk-explicit"),
    Some("http://explicit/v1"),
    "sk-explicit"
)]
#[case::absent_takes_environment(ENVIRONMENT, None, None, Some("http://env/v1"), "sk-env")]
#[case::empty_takes_environment(ENVIRONMENT, Some(""), Some(""), Some("http://env/v1"), "sk-env")]
#[case::nothing_anywhere(&[], None, None, None, "")]
#[case::empty_without_environment(&[], Some(""), Some(""), None, "")]
fn resolves_explicit_empty_and_absent_values(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] base: Option<&str>,
    #[case] key: &str,
) {
    let key_override = explicit_key.map(SecretString::from);
    let (resolved_base, resolved_key) =
        settings(environment).resolve(explicit_base, key_override.as_ref());
    assert_eq!(resolved_base.as_deref(), base);
    assert_eq!(resolved_key.expose_secret(), key);
}
