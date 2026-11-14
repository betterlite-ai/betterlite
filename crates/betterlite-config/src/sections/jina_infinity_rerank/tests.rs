#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::jina_infinity_rerank::Settings;

const GLOBAL_KEY: &str = "litellm_settings:\n  infinity_key: global\n";

fn load(variables: &[(&str, &str)], source: &str) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .jina_infinity_rerank)
}

fn revealed(key: Option<&SecretString>) -> Option<&str> {
    key.map(|secret| secret.expose_secret())
}

#[rstest]
#[case::environment(&[("INFINITY_API_KEY", "env")], "", Some("env"), None)]
#[case::empty_environment(&[("INFINITY_API_KEY", "")], "", None, None)]
#[case::global(&[], GLOBAL_KEY, None, Some("global"))]
#[case::empty_global(&[], "litellm_settings:\n  infinity_key: ''\n", None, Some(""))]
#[case::referenced_global(
    &[("SECRET", "from-env")],
    "litellm_settings:\n  infinity_key: os.environ/SECRET\n",
    None,
    Some("from-env")
)]
#[case::both(&[("INFINITY_API_KEY", "env")], GLOBAL_KEY, Some("env"), Some("global"))]
#[case::neither(&[], "", None, None)]
#[case::other_variables_are_not_read(&[("JINA_AI_API_KEY", "jina")], "", None, None)]
fn the_environment_and_the_global_key_load(
    #[case] variables: &[(&str, &str)],
    #[case] source: &str,
    #[case] from_environment: Option<&str>,
    #[case] global: Option<&str>,
) {
    let settings = load(variables, source).unwrap();
    assert_eq!(
        revealed(settings.infinity_api_key.as_ref()),
        from_environment
    );
    assert_eq!(revealed(settings.infinity_key.as_ref()), global);
}

#[test]
fn a_missing_referenced_variable_is_an_error_naming_the_path() {
    let error = load(
        &[],
        "litellm_settings:\n  infinity_key: os.environ/ABSENT\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("litellm_settings.infinity_key"));
}

#[rstest]
#[case::explicit_wins(Some("explicit"), Some("env"), Some("global"), Some("explicit"))]
#[case::explicit_empty_stays(Some(""), Some("env"), Some("global"), Some(""))]
#[case::environment_over_global(None, Some("env"), Some("global"), Some("env"))]
#[case::empty_environment_falls_through(None, Some(""), Some("global"), Some("global"))]
#[case::global_alone(None, None, Some("global"), Some("global"))]
#[case::empty_global_is_kept(None, None, Some(""), Some(""))]
#[case::nothing(None, None, None, None)]
fn the_key_resolves_in_order(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] global: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let settings = Settings {
        infinity_api_key: environment.map(|key| SecretString::from(key.to_owned())),
        infinity_key: global.map(|key| SecretString::from(key.to_owned())),
    };
    let explicit_key = explicit.map(|key| SecretString::from(key.to_owned()));
    let resolved = settings.resolve_api_key(explicit_key.as_ref());
    assert_eq!(revealed(resolved.as_ref()), expected);
}
