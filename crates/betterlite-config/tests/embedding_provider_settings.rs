#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::embedding_provider_settings::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

const MODULE: &str = "litellm_settings:\n  cohere_key: module-key\n  api_key: global-key\n  api_base: https://global.test\n  headers:\n    X-Org: litellm\n";

type Environment<'case> = &'case [(&'case str, &'case str)];

fn settings(environment: Environment<'_>, source: &str) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .embedding_provider_settings)
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[rstest]
#[case::long_alias(&[("COHERE_API_KEY", "long")], "", None, Some("long"), None)]
#[case::short_alias(&[("CO_API_KEY", "short")], "", None, None, Some("short"))]
#[case::both_aliases(&[("COHERE_API_KEY", "long"), ("CO_API_KEY", "short")], "", None, Some("long"), Some("short"))]
#[case::empty_long_alias(&[("COHERE_API_KEY", "")], "", None, None, None)]
#[case::missing(&[], "", None, None, None)]
#[case::module_values(&[], MODULE, Some("module-key"), None, None)]
#[case::empty_module_key(&[], "litellm_settings:\n  cohere_key: ''\n", Some(""), None, None)]
fn the_cohere_environment_aliases_and_module_key_load(
    #[case] environment: Environment<'_>,
    #[case] source: &str,
    #[case] expected_cohere_key: Option<&str>,
    #[case] expected_long: Option<&str>,
    #[case] expected_short: Option<&str>,
) {
    let loaded = settings(environment, source).expect("the config loads");
    assert_eq!(revealed(loaded.cohere_key.as_ref()), expected_cohere_key);
    assert_eq!(revealed(loaded.cohere_api_key.as_ref()), expected_long);
    assert_eq!(revealed(loaded.co_api_key.as_ref()), expected_short);
}

#[derive(Clone, Copy)]
enum Chain {
    Key,
    Base,
}

fn resolved(
    chain: Chain,
    environment: Environment<'_>,
    source: &str,
    explicit: Option<&str>,
) -> Option<String> {
    let settings = settings(environment, source).expect("the config loads");
    match chain {
        Chain::Key => {
            let key = explicit.map(|text| SecretString::from(text.to_owned()));
            settings
                .resolve_api_key(key.as_ref())
                .map(|secret| secret.expose_secret().to_owned())
        }
        Chain::Base => settings.resolve_api_base(explicit),
    }
}

#[rstest]
#[case::key_explicit_wins(Chain::Key, &[("COHERE_API_KEY", "long")], MODULE, Some("explicit"), Some("explicit"))]
#[case::key_module_key_before_the_environment(Chain::Key, &[("COHERE_API_KEY", "long")], MODULE, None, Some("module-key"))]
#[case::key_long_alias_before_the_short_one(Chain::Key, &[("COHERE_API_KEY", "long"), ("CO_API_KEY", "short")], "", None, Some("long"))]
#[case::key_short_alias_before_the_api_key(Chain::Key, &[("CO_API_KEY", "short")], "", None, Some("short"))]
#[case::key_the_global_key_is_last(Chain::Key, &[], "litellm_settings:\n  api_key: global-key\n", None, Some("global-key"))]
#[case::key_empty_long_alias_falls_through(Chain::Key, &[("COHERE_API_KEY", ""), ("CO_API_KEY", "short")], "", None, Some("short"))]
#[case::key_explicit_empty_falls_through(Chain::Key, &[("CO_API_KEY", "short")], "", Some(""), Some("short"))]
#[case::key_the_global_empty_key_is_kept(Chain::Key, &[], "litellm_settings:\n  api_key: ''\n", None, Some(""))]
#[case::key_missing_everywhere(Chain::Key, &[], "", None, None)]
#[case::base_explicit_wins(Chain::Base, &[("COHERE_API_BASE", "https://env.test")], MODULE, Some("https://explicit.test"), Some("https://explicit.test"))]
#[case::base_the_module_base_before_the_environment(Chain::Base, &[("COHERE_API_BASE", "https://env.test")], MODULE, None, Some("https://global.test"))]
#[case::base_the_environment_base_is_last(Chain::Base, &[("COHERE_API_BASE", "https://env.test")], "", None, Some("https://env.test"))]
#[case::base_an_empty_environment_base_is_unset(Chain::Base, &[("COHERE_API_BASE", "")], "", None, None)]
#[case::base_an_explicit_empty_base_falls_through(Chain::Base, &[], "litellm_settings:\n  api_base: https://global.test\n", Some(""), Some("https://global.test"))]
#[case::base_an_empty_module_base_falls_through(Chain::Base, &[("COHERE_API_BASE", "https://env.test")], "litellm_settings:\n  api_base: ''\n", None, Some("https://env.test"))]
#[case::base_no_default_applies_here(Chain::Base, &[], "", None, None)]
fn the_cohere_key_and_base_chains_are_ordered(
    #[case] chain: Chain,
    #[case] environment: Environment<'_>,
    #[case] source: &str,
    #[case] explicit: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        resolved(chain, environment, source, explicit).as_deref(),
        expected
    );
}

#[test]
fn an_empty_cohere_base_is_unset_before_and_after_the_settings() {
    let loaded = settings(&[("COHERE_API_BASE", "")], "").expect("the config loads");
    assert!(loaded.cohere_api_base.is_none());
    let direct = Settings {
        cohere_api_key: None,
        co_api_key: None,
        cohere_api_base: Some(String::new()),
        cohere_key: None,
        api_key: None,
        api_base: None,
        headers: BTreeMap::new(),
    };
    assert_eq!(direct.resolve_api_base(None), None);
}

fn source_headers(source: &str) -> BTreeMap<String, String> {
    settings(&[], source).expect("the config loads").headers
}

#[test]
fn the_litellm_settings_headers_load() {
    assert_eq!(
        source_headers(MODULE),
        BTreeMap::from([(String::from("X-Org"), String::from("litellm"))])
    );
    assert!(source_headers("").is_empty());
}

#[test]
fn an_environment_reference_in_the_module_values_loads_the_variable() {
    let loaded = settings(
        &[("BASE", "https://from-env.test"), ("KEY", "from-env")],
        "litellm_settings:\n  api_base: os.environ/BASE\n  cohere_key: os.environ/KEY\n",
    )
    .expect("the config loads");
    assert_eq!(loaded.api_base.as_deref(), Some("https://from-env.test"));
    assert_eq!(revealed(loaded.cohere_key.as_ref()), Some("from-env"));
    let error = settings(
        &[],
        "litellm_settings:\n  api_base: os.environ/MISSING_BASE\n",
    )
    .expect_err("the reference cannot resolve");
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolved_headers(
    supplied: Option<&[(&str, &str)]>,
    extra: &[(&str, &str)],
) -> BTreeMap<String, String> {
    let settings = settings(&[], MODULE).expect("the config loads");
    settings.resolve_headers(supplied.map(map).as_ref(), &map(extra))
}

#[rstest]
#[case::the_global_headers_alone(None, &[], &[("X-Org", "litellm")])]
#[case::the_supplied_headers_win(Some(&[("X-Org", "caller")][..]), &[], &[("X-Org", "caller")])]
#[case::an_empty_supplied_map_falls_back(Some(&[][..]), &[], &[("X-Org", "litellm")])]
#[case::the_extra_headers_overlay_the_choice(Some(&[("Accept", "text/plain")][..]), &[("Accept", "application/json")], &[("Accept", "application/json")])]
#[case::the_extra_headers_overlay_the_globals(None, &[("X-Org", "extra")], &[("X-Org", "extra")])]
fn the_chat_headers_are_supplied_then_global_then_extra(
    #[case] supplied: Option<&[(&str, &str)]>,
    #[case] extra: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    assert_eq!(resolved_headers(supplied, extra), map(expected));
}
