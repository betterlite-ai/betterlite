#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::studio_identity::{DEFAULT_API_BASE, Settings};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .studio_identity)
}

fn revealed(key: Option<&SecretString>) -> Option<&str> {
    key.map(ExposeSecret::expose_secret)
}

#[test]
fn studio_identity_settings_default_to_nothing() {
    let settings = load("", &[]).expect("an empty config loads");
    assert!(settings.environment_key.is_none());
    assert!(settings.generic_api_key.is_none());
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
}

#[rstest]
#[case::google_key(&[("GOOGLE_API_KEY", "google-key")], Some("google-key"))]
#[case::gemini_key(&[("GEMINI_API_KEY", "gemini-key")], Some("gemini-key"))]
#[case::google_precedes_gemini(
    &[("GOOGLE_API_KEY", "google-key"), ("GEMINI_API_KEY", "gemini-key")],
    Some("google-key")
)]
#[case::empty_google_falls_through(
    &[("GOOGLE_API_KEY", ""), ("GEMINI_API_KEY", "gemini-key")],
    Some("gemini-key")
)]
#[case::empty_values_are_unset(&[("GOOGLE_API_KEY", ""), ("GEMINI_API_KEY", "")], None)]
#[case::absent(&[], None)]
fn studio_identity_settings_load_environment_keys(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load("", environment).expect("the config loads");
    assert_eq!(revealed(settings.environment_key.as_ref()), expected);
}

#[rstest]
#[case::module_key("litellm_settings:\n  api_key: generic-key\n", &[], Some("generic-key"))]
#[case::environment_reference(
    "litellm_settings:\n  api_key: os.environ/MY_GENERIC_KEY\n",
    &[("MY_GENERIC_KEY", "referenced")],
    Some("referenced")
)]
#[case::absent("", &[], None)]
fn studio_identity_settings_load_the_litellm_key(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load(source, environment).expect("the config loads");
    assert_eq!(revealed(settings.generic_api_key.as_ref()), expected);
}

#[test]
fn studio_identity_settings_reject_a_missing_environment_reference() {
    let error = load("litellm_settings:\n  api_key: os.environ/ABSENT\n", &[])
        .expect_err("the reference names no variable");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

#[rstest]
#[case::set(&[("GEMINI_API_BASE", "https://proxy.example")], "https://proxy.example")]
#[case::empty_is_unset(&[("GEMINI_API_BASE", "")], "https://generativelanguage.googleapis.com")]
#[case::missing(&[], "https://generativelanguage.googleapis.com")]
fn studio_identity_settings_load_the_api_base(
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = load("", environment).expect("the config loads");
    assert_eq!(settings.api_base, expected);
}

#[test]
fn studio_identity_settings_hide_keys_from_debug() {
    let settings = load(
        "",
        &[
            ("GOOGLE_API_KEY", "sk-google"),
            ("GEMINI_API_KEY", "sk-gemini"),
        ],
    )
    .expect("the config loads");
    let debug = format!("{settings:?}");
    assert!(!debug.contains("sk-google"));
    assert!(!debug.contains("sk-gemini"));
}
