#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::hosted_chat_settings::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

fn success(source: &str, environment: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    }
    .load(source)
    .expect("the config loads")
    .sections
    .hosted_chat_settings
}

fn settings(environment: &[(&str, &str)]) -> Settings {
    success("", environment)
}

fn revealed(key: Option<&SecretString>) -> Option<String> {
    key.map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn every_alias_defaults_absent() {
    let loaded = settings(&[]);
    assert_eq!(loaded.featherless_ai_api_base, None);
    assert_eq!(loaded.featherless_api_base, None);
    assert_eq!(revealed(loaded.featherless_ai_api_key.as_ref()), None);
    assert_eq!(revealed(loaded.featherless_api_key.as_ref()), None);
    assert_eq!(loaded.aiml_api_base, None);
    assert_eq!(revealed(loaded.aiml_api_key.as_ref()), None);
    assert!(!loaded.drop_params);
}

#[rstest]
#[case::absent(&[], None, None, None, None)]
#[case::primary_base(
    &[("FEATHERLESS_AI_API_BASE", "https://primary.example")],
    Some("https://primary.example"),
    None,
    None,
    None
)]
#[case::secondary_base(
    &[("FEATHERLESS_API_BASE", "https://secondary.example")],
    None,
    Some("https://secondary.example"),
    None,
    None
)]
#[case::both_bases(
    &[
        ("FEATHERLESS_AI_API_BASE", "https://primary.example"),
        ("FEATHERLESS_API_BASE", "https://secondary.example"),
    ],
    Some("https://primary.example"),
    Some("https://secondary.example"),
    None,
    None
)]
#[case::primary_key(&[("FEATHERLESS_AI_API_KEY", "sk-primary")], None, None, Some("sk-primary"), None)]
#[case::secondary_key(&[("FEATHERLESS_API_KEY", "sk-secondary")], None, None, None, Some("sk-secondary"))]
#[case::empty_values_are_unset(
    &[
        ("FEATHERLESS_AI_API_BASE", ""),
        ("FEATHERLESS_API_BASE", ""),
        ("FEATHERLESS_AI_API_KEY", ""),
        ("FEATHERLESS_API_KEY", ""),
    ],
    None,
    None,
    None,
    None
)]
fn featherless_aliases_load_independently(
    #[case] environment: &[(&str, &str)],
    #[case] primary_base: Option<&str>,
    #[case] secondary_base: Option<&str>,
    #[case] primary_key: Option<&str>,
    #[case] secondary_key: Option<&str>,
) {
    let loaded = settings(environment);
    assert_eq!(loaded.featherless_ai_api_base.as_deref(), primary_base);
    assert_eq!(loaded.featherless_api_base.as_deref(), secondary_base);
    assert_eq!(
        revealed(loaded.featherless_ai_api_key.as_ref()).as_deref(),
        primary_key
    );
    assert_eq!(
        revealed(loaded.featherless_api_key.as_ref()).as_deref(),
        secondary_key
    );
}

#[rstest]
#[case::absent(&[], None, None)]
#[case::base(&[("AIML_API_BASE", "https://aiml.example/v1")], Some("https://aiml.example/v1"), None)]
#[case::key(&[("AIML_API_KEY", "sk-aiml")], None, Some("sk-aiml"))]
#[case::both(
    &[("AIML_API_BASE", "https://aiml.example/v1"), ("AIML_API_KEY", "sk-aiml")],
    Some("https://aiml.example/v1"),
    Some("sk-aiml")
)]
#[case::empty_values_are_unset(&[("AIML_API_BASE", ""), ("AIML_API_KEY", "")], None, None)]
fn aiml_aliases_load(
    #[case] environment: &[(&str, &str)],
    #[case] base: Option<&str>,
    #[case] key: Option<&str>,
) {
    let loaded = settings(environment);
    assert_eq!(loaded.aiml_api_base.as_deref(), base);
    assert_eq!(revealed(loaded.aiml_api_key.as_ref()).as_deref(), key);
}

#[test]
fn the_image_only_aimlapi_key_is_not_a_chat_credential() {
    let loaded = settings(&[("AIMLAPI_KEY", "sk-image-only")]);
    assert_eq!(revealed(loaded.aiml_api_key.as_ref()), None);
}

#[rstest]
#[case::default(&[], "", false)]
#[case::environment_on(&[("LITELLM_DROP_PARAMS", "true")], "", true)]
#[case::environment_off(&[("LITELLM_DROP_PARAMS", "off")], "", false)]
#[case::environment_empty(&[("LITELLM_DROP_PARAMS", "")], "", false)]
#[case::invalid_environment_turns_on(&[("LITELLM_DROP_PARAMS", "always")], "", true)]
#[case::config_on(&[], "litellm_settings:\n  drop_params: true\n", true)]
#[case::config_off(&[], "litellm_settings:\n  drop_params: false\n", false)]
#[case::config_wins_over_environment(&[("LITELLM_DROP_PARAMS", "true")], "litellm_settings:\n  drop_params: false\n", false)]
fn the_global_drop_flag_is_loaded(
    #[case] environment: &[(&str, &str)],
    #[case] source: &str,
    #[case] expected: bool,
) {
    assert_eq!(success(source, environment).drop_params, expected);
}
