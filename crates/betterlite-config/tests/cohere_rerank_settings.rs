#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::cohere_rerank_settings::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

const MODULE_KEY: &str = "litellm_settings:\n  cohere_key: module-key\n";

fn settings(environment: &[(&str, &str)], source: &str) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .cohere_rerank_settings)
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

fn loaded(
    environment: &[(&str, &str)],
    source: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let settings = settings(environment, source).expect("the config loads");
    (
        revealed(settings.cohere_api_key.as_ref()).map(str::to_owned),
        revealed(settings.co_api_key.as_ref()).map(str::to_owned),
        revealed(settings.cohere_key.as_ref()).map(str::to_owned),
    )
}

#[rstest]
#[case::long_alias(&[("COHERE_API_KEY", "long")], "", Some("long"), None, None)]
#[case::short_alias(&[("CO_API_KEY", "short")], "", None, Some("short"), None)]
#[case::both_aliases(&[("COHERE_API_KEY", "long"), ("CO_API_KEY", "short")], "", Some("long"), Some("short"), None)]
#[case::empty_long_alias(&[("COHERE_API_KEY", "")], "", None, None, None)]
#[case::missing(&[], "", None, None, None)]
#[case::module_key(&[], MODULE_KEY, None, None, Some("module-key"))]
#[case::empty_module_key(&[], "litellm_settings:\n  cohere_key: ''\n", None, None, Some(""))]
fn the_two_environment_aliases_and_the_module_key_load(
    #[case] environment: &[(&str, &str)],
    #[case] source: &str,
    #[case] long: Option<&str>,
    #[case] short: Option<&str>,
    #[case] module: Option<&str>,
) {
    let (loaded_long, loaded_short, loaded_module) = loaded(environment, source);
    assert_eq!(loaded_long.as_deref(), long);
    assert_eq!(loaded_short.as_deref(), short);
    assert_eq!(loaded_module.as_deref(), module);
}

fn resolved(environment: &[(&str, &str)], source: &str, explicit: Option<&str>) -> Option<String> {
    let settings = settings(environment, source).expect("the config loads");
    let key = explicit.map(|text| SecretString::from(text.to_owned()));
    settings
        .resolve_api_key(key.as_ref())
        .map(|secret| secret.expose_secret().to_owned())
}

#[rstest]
#[case::long_alias_wins(&[("COHERE_API_KEY", "long"), ("CO_API_KEY", "short")], MODULE_KEY, None, Some("long"))]
#[case::short_alias_before_the_module_key(&[("CO_API_KEY", "short")], MODULE_KEY, None, Some("short"))]
#[case::module_key_last(&[], MODULE_KEY, None, Some("module-key"))]
#[case::empty_long_alias_falls_through(&[("COHERE_API_KEY", ""), ("CO_API_KEY", "short")], MODULE_KEY, None, Some("short"))]
#[case::empty_aliases_use_the_module_key(&[("COHERE_API_KEY", ""), ("CO_API_KEY", "")], MODULE_KEY, None, Some("module-key"))]
#[case::explicit_key_wins(&[("COHERE_API_KEY", "long")], MODULE_KEY, Some("explicit"), Some("explicit"))]
#[case::explicit_empty_key_is_kept(&[("COHERE_API_KEY", "long")], MODULE_KEY, Some(""), Some(""))]
#[case::missing_everywhere(&[], "", None, None)]
#[case::empty_aliases_without_the_module_key(&[("COHERE_API_KEY", ""), ("CO_API_KEY", "")], "", None, None)]
fn the_rerank_key_order_is_long_alias_short_alias_module(
    #[case] environment: &[(&str, &str)],
    #[case] source: &str,
    #[case] explicit: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(resolved(environment, source, explicit).as_deref(), expected);
}

#[test]
fn an_empty_module_key_alone_is_kept() {
    let key = resolved(&[], "litellm_settings:\n  cohere_key: ''\n", None);
    assert_eq!(key.as_deref(), Some(""));
}

#[test]
fn an_environment_reference_in_the_module_key_loads_the_variable() {
    let key = resolved(
        &[("MODULE_KEY", "from-env")],
        "litellm_settings:\n  cohere_key: os.environ/MODULE_KEY\n",
        None,
    );
    assert_eq!(key.as_deref(), Some("from-env"));
}

#[test]
fn a_reference_to_an_unset_variable_fails_the_load() {
    let error = settings(
        &[],
        "litellm_settings:\n  cohere_key: os.environ/MODULE_KEY\n",
    )
    .expect_err("the reference cannot resolve");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}
