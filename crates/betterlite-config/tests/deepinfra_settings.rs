#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::deepinfra_settings::{
    DEFAULT_API_BASE, DEFAULT_MINIMUM_TEMPERATURE, Settings,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

fn settings(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .deepinfra_settings
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn exposed(key: Option<&SecretString>) -> Option<String> {
    key.map(|value| value.expose_secret().to_owned())
}

#[rstest]
#[case::both_keys(
    &[("DEEPINFRA_API_BASE", "https://env.example"), ("DEEPINFRA_API_KEY", "env-key")],
    Some("https://env.example"),
    Some("env-key")
)]
#[case::base_only(&[("DEEPINFRA_API_BASE", "https://env.example")], Some("https://env.example"), None)]
#[case::key_only(&[("DEEPINFRA_API_KEY", "env-key")], None, Some("env-key"))]
#[case::nothing(&[], None, None)]
#[case::empty_values(
    &[("DEEPINFRA_API_BASE", ""), ("DEEPINFRA_API_KEY", "")],
    None,
    None
)]
fn deepinfra_environment_aliases_load(
    #[case] environment: &[(&str, &str)],
    #[case] expected_base: Option<&str>,
    #[case] expected_key: Option<&str>,
) {
    let loaded = settings("", environment);
    assert_eq!(loaded.api_base.as_deref(), expected_base);
    assert_eq!(exposed(loaded.api_key.as_ref()).as_deref(), expected_key);
}

#[rstest]
#[case::unset("", DEFAULT_MINIMUM_TEMPERATURE)]
#[case::value("0.5", 0.5)]
#[case::integer_text("2", 2.0)]
#[case::padded("  0.25  ", 0.25)]
#[case::empty("", DEFAULT_MINIMUM_TEMPERATURE)]
fn deepinfra_minimum_temperature_reads_the_environment(#[case] text: &str, #[case] expected: f64) {
    let loaded = settings("", &[("MIN_NON_ZERO_TEMPERATURE", text)]);
    assert!((loaded.min_non_zero_temperature - expected).abs() < f64::EPSILON);
}

#[test]
fn deepinfra_minimum_temperature_rejects_a_non_number() {
    let lookup = BTreeMap::from([(
        String::from("MIN_NON_ZERO_TEMPERATURE"),
        String::from("warm"),
    )]);
    let loaded = ConfigLoader { lookup }.load("");
    assert!(loaded.is_err());
}

#[rstest]
#[case::default("", &[], false)]
#[case::yaml_on("litellm_settings:\n  drop_params: true\n", &[], true)]
#[case::yaml_off("litellm_settings:\n  drop_params: false\n", &[], false)]
#[case::environment_on("", &[("LITELLM_DROP_PARAMS", "yes")], true)]
#[case::environment_off("", &[("LITELLM_DROP_PARAMS", "0")], false)]
#[case::empty_environment("", &[("LITELLM_DROP_PARAMS", "")], false)]
#[case::yaml_wins_over_environment(
    "litellm_settings:\n  drop_params: false\n",
    &[("LITELLM_DROP_PARAMS", "true")],
    false
)]
fn deepinfra_drop_params_defaults_to_false(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: bool,
) {
    assert_eq!(settings(source, environment).drop_params, expected);
}

#[rstest]
#[case::explicit_wins(
    &[("DEEPINFRA_API_BASE", "https://env.example"), ("DEEPINFRA_API_KEY", "env-key")],
    Some("https://explicit.example"),
    Some("explicit-key"),
    "https://explicit.example",
    Some("explicit-key")
)]
#[case::environment(
    &[("DEEPINFRA_API_BASE", "https://env.example"), ("DEEPINFRA_API_KEY", "env-key")],
    None,
    None,
    "https://env.example",
    Some("env-key")
)]
#[case::empty_falls_through(
    &[("DEEPINFRA_API_BASE", "https://env.example"), ("DEEPINFRA_API_KEY", "env-key")],
    Some(""),
    Some(""),
    "https://env.example",
    Some("env-key")
)]
#[case::default_base(
    &[],
    None,
    None,
    DEFAULT_API_BASE,
    None
)]
#[case::empty_without_environment(&[], Some(""), Some(""), DEFAULT_API_BASE, None)]
fn deepinfra_chat_identity_follows_the_pinned_fallbacks(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected_base: &str,
    #[case] expected_key: Option<&str>,
) {
    let supplied = explicit_key.map(secret);
    let (base, key) = settings("", environment).resolve(explicit_base, supplied.as_ref());
    assert_eq!(base, expected_base);
    assert_eq!(exposed(key.as_ref()).as_deref(), expected_key);
}

#[rstest]
#[case::caller_key_wins(&[("DEEPINFRA_API_KEY", "env-key")], Some("caller"), Some("caller"))]
#[case::empty_caller_key_is_kept(&[("DEEPINFRA_API_KEY", "env-key")], Some(""), Some(""))]
#[case::environment_fills_none(&[("DEEPINFRA_API_KEY", "env-key")], None, Some("env-key"))]
#[case::nothing_anywhere(&[], None, None)]
fn deepinfra_rerank_key_only_falls_back_for_a_missing_key(
    #[case] environment: &[(&str, &str)],
    #[case] explicit_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let supplied = explicit_key.map(secret);
    let key = settings("", environment).rerank_key(supplied.as_ref());
    assert_eq!(exposed(key.as_ref()).as_deref(), expected);
}
