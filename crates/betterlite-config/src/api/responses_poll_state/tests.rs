#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    DEFAULT_POLLING_TTL, NATIVE_BACKGROUND_MODE_PATH, POLLING_VIA_CACHE_PATH, PollingSelector,
    PollingSettings, TTL_PATH,
};
use crate::loader::ConfigError;

enum Expected {
    Settings(PollingSettings),
    InvalidInteger(&'static str),
    InvalidScalar(&'static str),
    MissingEnvironment(&'static str, &'static str),
}

fn background(body: &str) -> String {
    format!("litellm_settings:\n  responses:\n    background_mode:\n{body}")
}

fn lookup(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn integer_path(result: Result<PollingSettings, ConfigError>) -> String {
    let error = result.unwrap_err();
    let ConfigError::InvalidInteger { path, .. } = &error else {
        panic!("expected an invalid integer, got {error:?}");
    };
    path.clone()
}

fn scalar_path(result: Result<PollingSettings, ConfigError>) -> String {
    let error = result.unwrap_err();
    let ConfigError::InvalidScalar { path, .. } = &error else {
        panic!("expected an invalid scalar, got {error:?}");
    };
    path.clone()
}

#[rstest]
#[case::absent_keys("", &[], Expected::Settings(PollingSettings::default()))]
#[case::all_override(
    "      polling_via_cache: \"all\"\n      native_background_mode: [gpt-4o]\n      ttl: 17\n",
    &[],
    Expected::Settings(PollingSettings {
        polling_via_cache: PollingSelector::All,
        native_background_mode: vec!["gpt-4o".to_owned()],
        ttl: 17,
    })
)]
#[case::provider_selector(
    "      polling_via_cache: [openai]\n",
    &[],
    Expected::Settings(PollingSettings {
        polling_via_cache: PollingSelector::Providers(vec!["openai".to_owned()]),
        ..PollingSettings::default()
    })
)]
#[case::zero_ttl_is_kept(
    "      ttl: 0\n",
    &[],
    Expected::Settings(PollingSettings {
        ttl: 0,
        ..PollingSettings::default()
    })
)]
#[case::negative_ttl_is_kept(
    "      ttl: -1\n",
    &[],
    Expected::Settings(PollingSettings {
        ttl: -1,
        ..PollingSettings::default()
    })
)]
#[case::boolean_selector_selects_nothing(
    "      polling_via_cache: true\n",
    &[],
    Expected::Settings(PollingSettings {
        polling_via_cache: PollingSelector::Unrecognized,
        ..PollingSettings::default()
    })
)]
#[case::other_selector_selects_nothing(
    "      polling_via_cache: \"other\"\n",
    &[],
    Expected::Settings(PollingSettings {
        polling_via_cache: PollingSelector::Unrecognized,
        ..PollingSettings::default()
    })
)]
#[case::empty_selector_selects_nothing(
    "      polling_via_cache: \"\"\n",
    &[],
    Expected::Settings(PollingSettings::default())
)]
#[case::environment_references(
    "      polling_via_cache: os.environ/POLLING\n      native_background_mode: [os.environ/NATIVE]\n      ttl: os.environ/TTL\n",
    &[("POLLING", "all"), ("NATIVE", "gpt-4o"), ("TTL", "17")],
    Expected::Settings(PollingSettings {
        polling_via_cache: PollingSelector::All,
        native_background_mode: vec!["gpt-4o".to_owned()],
        ttl: 17,
    })
)]
#[case::empty_environment_ttl_is_unset(
    "      ttl: os.environ/TTL\n",
    &[("TTL", "")],
    Expected::Settings(PollingSettings::default())
)]
#[case::text_ttl_fails(
    "      ttl: oops\n",
    &[],
    Expected::InvalidInteger(TTL_PATH)
)]
#[case::float_ttl_fails(
    "      ttl: 1.5\n",
    &[],
    Expected::InvalidScalar(TTL_PATH)
)]
#[case::scalar_native_modes_fail(
    "      native_background_mode: 7\n",
    &[],
    Expected::InvalidScalar(NATIVE_BACKGROUND_MODE_PATH)
)]
#[case::object_selector_fails(
    "      polling_via_cache: {openai: true}\n",
    &[],
    Expected::InvalidScalar(POLLING_VIA_CACHE_PATH)
)]
#[case::non_text_provider_fails(
    "      polling_via_cache: [1]\n",
    &[],
    Expected::InvalidScalar(POLLING_VIA_CACHE_PATH)
)]
#[case::missing_environment_fails(
    "      ttl: os.environ/ABSENT\n",
    &[],
    Expected::MissingEnvironment("ABSENT", TTL_PATH)
)]
fn loads_polling_settings(
    #[case] body: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Expected,
) {
    let result = super::loads_polling_settings(&background(body), &lookup(environment));
    match expected {
        Expected::Settings(want) => assert_eq!(result.unwrap(), want),
        Expected::InvalidInteger(path) => assert_eq!(integer_path(result), path),
        Expected::InvalidScalar(path) => assert_eq!(scalar_path(result), path),
        Expected::MissingEnvironment(variable, path) => {
            let ConfigError::MissingEnvironment {
                variable: found,
                path: found_path,
            } = result.unwrap_err()
            else {
                panic!("expected a missing environment error");
            };
            assert_eq!((found.as_str(), found_path.as_str()), (variable, path));
        }
    }
}

#[rstest]
fn defaults_match_the_proxy_settings_defaults() {
    assert_eq!(PollingSettings::default().ttl, DEFAULT_POLLING_TTL);
}
