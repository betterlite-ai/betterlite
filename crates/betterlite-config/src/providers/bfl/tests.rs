#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::types::RouterSettings;

const DEFAULT_API_BASE: &str = "https://api.bfl.ai";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for (name, value) in entries {
        found.insert((*name).to_owned(), (*value).to_owned());
    }
    found
}

#[rstest]
#[case::defaults(&[], None, DEFAULT_API_BASE)]
#[case::both_keys(&[("BFL_API_KEY", "bfl"), ("BLACK_FOREST_LABS_API_KEY", "legacy")], Some("bfl"), DEFAULT_API_BASE)]
#[case::fallback_key(&[("BLACK_FOREST_LABS_API_KEY", "legacy")], Some("legacy"), DEFAULT_API_BASE)]
#[case::custom_base(&[("BFL_API_KEY", "bfl"), ("BFL_API_BASE", "https://proxy.test/")], Some("bfl"), "https://proxy.test/")]
#[case::empty_values(&[("BFL_API_KEY", ""), ("BLACK_FOREST_LABS_API_KEY", ""), ("BFL_API_BASE", "")], None, DEFAULT_API_BASE)]
#[case::boolean_lookalikes(&[("BFL_API_KEY", "true"), ("BLACK_FOREST_LABS_API_KEY", " FALSE "), ("BFL_API_BASE", "true")], None, DEFAULT_API_BASE)]
#[case::whitespace_is_kept(&[("BFL_API_KEY", "  "), ("BFL_API_BASE", "  ")], Some("  "), "  ")]
fn bfl_generation_settings(
    #[case] environment: &[(&str, &str)],
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let (api_key, api_base) = RouterSettings::bfl_generation_settings(&lookup(environment));

    match expected_key {
        Some(expected) => assert_eq!(api_key.as_ref().unwrap().expose_secret(), expected),
        None => assert!(api_key.is_none()),
    }
    assert_eq!(api_base, expected_base);
}
