#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

#[derive(Debug)]
enum Expected {
    Allowed(bool),
    InvalidScalar,
    MissingEnvironment,
    BadEnvironment,
}

#[rstest]
#[case::unset_defaults_on("", &[], Expected::Allowed(true))]
#[case::empty_section_defaults_on("litellm_settings: {}\n", &[], Expected::Allowed(true))]
#[case::explicit_null_defaults_on(
    "litellm_settings:\n  allow_dynamic_callback_disabling: null\n",
    &[],
    Expected::Allowed(true)
)]
#[case::explicit_false(
    "litellm_settings:\n  allow_dynamic_callback_disabling: false\n",
    &[],
    Expected::Allowed(false)
)]
#[case::explicit_true(
    "litellm_settings:\n  allow_dynamic_callback_disabling: true\n",
    &[],
    Expected::Allowed(true)
)]
#[case::environment_false(
    "litellm_settings:\n  allow_dynamic_callback_disabling: os.environ/CALLBACK_DISABLING\n",
    &[("CALLBACK_DISABLING", "False")],
    Expected::Allowed(false)
)]
#[case::environment_true(
    "litellm_settings:\n  allow_dynamic_callback_disabling: os.environ/CALLBACK_DISABLING\n",
    &[("CALLBACK_DISABLING", "on")],
    Expected::Allowed(true)
)]
#[case::empty_environment_defaults_on(
    "litellm_settings:\n  allow_dynamic_callback_disabling: os.environ/CALLBACK_DISABLING\n",
    &[("CALLBACK_DISABLING", "")],
    Expected::Allowed(true)
)]
#[case::unset_environment(
    "litellm_settings:\n  allow_dynamic_callback_disabling: os.environ/CALLBACK_DISABLING\n",
    &[],
    Expected::MissingEnvironment
)]
#[case::invalid_environment(
    "litellm_settings:\n  allow_dynamic_callback_disabling: os.environ/CALLBACK_DISABLING\n",
    &[("CALLBACK_DISABLING", "maybe")],
    Expected::BadEnvironment
)]
#[case::plain_text(
    "litellm_settings:\n  allow_dynamic_callback_disabling: \"false\"\n",
    &[],
    Expected::InvalidScalar
)]
#[case::number(
    "litellm_settings:\n  allow_dynamic_callback_disabling: 0\n",
    &[],
    Expected::InvalidScalar
)]
#[case::sequence(
    "litellm_settings:\n  allow_dynamic_callback_disabling: [true]\n",
    &[],
    Expected::InvalidScalar
)]
fn loads_settings(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Expected,
) {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loaded = ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.request_setup_options);
    match (loaded, expected) {
        (Ok(settings), Expected::Allowed(allowed)) => assert_eq!(
            settings,
            Settings {
                allow_dynamic_callback_disabling: allowed
            }
        ),
        (Err(ConfigError::InvalidScalar { path, .. }), Expected::InvalidScalar) => {
            assert_eq!(path, "litellm_settings.allow_dynamic_callback_disabling");
        }
        (Err(ConfigError::MissingEnvironment { variable, path }), Expected::MissingEnvironment) => {
            assert_eq!(variable, "CALLBACK_DISABLING");
            assert_eq!(path, "litellm_settings.allow_dynamic_callback_disabling");
        }
        (Err(ConfigError::Environment { source }), Expected::BadEnvironment) => {
            assert!(
                source.to_string().contains("CALLBACK_DISABLING"),
                "{source}"
            );
        }
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
}
