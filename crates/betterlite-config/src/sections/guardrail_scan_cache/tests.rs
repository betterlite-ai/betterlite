#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::guardrail_scan_cache::Settings;

fn load(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load("")?
        .sections
        .guardrail_scan_cache)
}

#[rstest]
#[case::missing(&[], 86400)]
#[case::empty_is_unset(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", "")], 86400)]
#[case::blank_is_unset(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", "  ")], 86400)]
#[case::integer(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", "120")], 120)]
#[case::trimmed(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", " 45\n")], 45)]
#[case::zero(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", "0")], 0)]
#[case::negative_is_kept(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", "-5")], -5)]
fn ttl_is_read_from_the_environment(#[case] variables: &[(&str, &str)], #[case] expected: i64) {
    assert_eq!(load(variables).unwrap().ttl_seconds, expected);
}

#[rstest]
#[case::text("soon")]
#[case::fraction("1.5")]
#[case::overflow("99999999999999999999")]
fn ttl_that_is_not_an_integer_names_the_variable(#[case] value: &str) {
    let error = load(&[("GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS", value)]).unwrap_err();
    assert!(matches!(
        &error,
        ConfigError::InvalidInteger { path, .. }
            if path == "GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS"
    ));
}

#[test]
fn settings_default_matches_the_loaded_default() {
    assert_eq!(Settings::default(), load(&[]).unwrap());
}
