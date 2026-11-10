#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    DEFAULT_MODEL_COST_MAP_URL, LOCAL_MODEL_COST_MAP_ENV, MODEL_COST_MAP_URL_ENV, Raw, Settings,
};
use crate::loader::ConfigError;
use crate::sections::Section as _;

fn configure(entries: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let environment = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Raw::default().convert(&environment)
}

#[rstest]
#[case::unset(&[], false, DEFAULT_MODEL_COST_MAP_URL)]
#[case::empty_values(&[(LOCAL_MODEL_COST_MAP_ENV, ""), (MODEL_COST_MAP_URL_ENV, "")], false, DEFAULT_MODEL_COST_MAP_URL)]
#[case::local_true(&[(LOCAL_MODEL_COST_MAP_ENV, "True")], true, DEFAULT_MODEL_COST_MAP_URL)]
#[case::local_spelled_true(&[(LOCAL_MODEL_COST_MAP_ENV, " TRUE ")], true, DEFAULT_MODEL_COST_MAP_URL)]
#[case::local_one(&[(LOCAL_MODEL_COST_MAP_ENV, "1")], true, DEFAULT_MODEL_COST_MAP_URL)]
#[case::local_false(&[(LOCAL_MODEL_COST_MAP_ENV, "false")], false, DEFAULT_MODEL_COST_MAP_URL)]
#[case::local_zero(&[(LOCAL_MODEL_COST_MAP_ENV, "0")], false, DEFAULT_MODEL_COST_MAP_URL)]
#[case::configured_url(&[(MODEL_COST_MAP_URL_ENV, "https://catalog.test/model.json")], false, "https://catalog.test/model.json")]
fn catalog_refresh_tests_behavior(
    #[case] entries: &[(&str, &str)],
    #[case] local: bool,
    #[case] url: &str,
) {
    let settings = configure(entries).unwrap();
    assert_eq!(settings.local, local);
    assert_eq!(settings.url, url);
}

#[rstest]
#[case::unspellable("maybe")]
#[case::negative("-1")]
fn an_unreadable_local_flag_names_its_variable(#[case] value: &str) {
    let error = configure(&[(LOCAL_MODEL_COST_MAP_ENV, value)]).unwrap_err();
    assert!(error.to_string().contains(LOCAL_MODEL_COST_MAP_ENV));
}

#[test]
fn the_default_settings_keep_the_pinned_url() {
    assert_eq!(Settings::default().url, DEFAULT_MODEL_COST_MAP_URL);
    assert!(!Settings::default().local);
}
