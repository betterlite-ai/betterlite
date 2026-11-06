#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load(source)?.sections.xai_identity)
}

fn revealed(key: Option<&SecretString>) -> Option<&str> {
    key.map(ExposeSecret::expose_secret)
}

#[test]
fn xai_identity_settings_default_to_nothing() {
    let settings = load("", &[]).unwrap();
    assert!(settings.xai_key.is_none());
    assert!(settings.generic_api_key.is_none());
    assert!(settings.environment_key.is_none());
    assert!(settings.api_base.is_none());
}

#[rstest]
#[case::environment_key("", &[("XAI_API_KEY", "env-key")], (None, None, Some("env-key")))]
#[case::empty_environment_key_is_unset("", &[("XAI_API_KEY", "")], (None, None, None))]
#[case::module_values(
    "litellm_settings:\n  xai_key: module-key\n  api_key: generic-key\n",
    &[],
    (Some("module-key"), Some("generic-key"), None)
)]
#[case::environment_reference(
    "litellm_settings:\n  xai_key: os.environ/MY_XAI\n",
    &[("MY_XAI", "referenced")],
    (Some("referenced"), None, None)
)]
fn xai_identity_settings_keys(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<&str>, Option<&str>, Option<&str>),
) {
    let settings = load(source, environment).unwrap();
    let (xai_key, generic_key, environment_key) = expected;
    assert_eq!(revealed(settings.xai_key.as_ref()), xai_key);
    assert_eq!(revealed(settings.generic_api_key.as_ref()), generic_key);
    assert_eq!(revealed(settings.environment_key.as_ref()), environment_key);
}

#[rstest]
#[case::set(&[("XAI_API_BASE", "https://xai.example")], Some("https://xai.example"))]
#[case::empty_is_unset(&[("XAI_API_BASE", "")], None)]
#[case::missing(&[], None)]
fn xai_identity_settings_base(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(load("", environment).unwrap().api_base.as_deref(), expected);
}

#[test]
fn xai_identity_settings_reject_a_missing_environment_reference() {
    let error = load("litellm_settings:\n  api_key: os.environ/ABSENT\n", &[]).unwrap_err();
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

#[test]
fn xai_identity_settings_hide_keys_from_debug() {
    let settings = load("litellm_settings:\n  xai_key: sk-hidden\n", &[]).unwrap();
    assert!(!format!("{settings:?}").contains("sk-hidden"));
}
