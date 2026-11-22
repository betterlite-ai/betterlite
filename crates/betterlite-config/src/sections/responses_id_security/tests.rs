#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .responses_id_security
}

#[rstest]
#[case::flags_omitted("general_settings: {}", &[], (false, false, None))]
#[case::disable_true(
    "general_settings:\n  disable_responses_id_security: true",
    &[],
    (true, false, None)
)]
#[case::allow_true(
    "general_settings:\n  allow_unmanaged_response_ids: true",
    &[],
    (false, true, None)
)]
#[case::both_true(
    "general_settings:\n  disable_responses_id_security: true\n  allow_unmanaged_response_ids: true",
    &[],
    (true, true, None)
)]
#[case::flags_explicit_false(
    "general_settings:\n  disable_responses_id_security: false\n  allow_unmanaged_response_ids: false",
    &[],
    (false, false, None)
)]
#[case::keys_absent_no_owner(
    "general_settings:\n  disable_responses_id_security: true",
    &[],
    (true, false, None)
)]
#[case::master_key_fallback(
    "general_settings:\n  master_key: test-master",
    &[],
    (false, false, Some("test-master"))
)]
#[case::salt_wins_over_master(
    "general_settings:\n  master_key: test-master",
    &[("LITELLM_SALT_KEY", "test-salt")],
    (false, false, Some("test-salt"))
)]
#[case::salt_alone(
    "general_settings: {}",
    &[("LITELLM_SALT_KEY", "test-salt")],
    (false, false, Some("test-salt"))
)]
#[case::empty_salt_stays(
    "general_settings:\n  master_key: test-master",
    &[("LITELLM_SALT_KEY", "")],
    (false, false, Some(""))
)]
#[case::empty_master_key_stays(
    "general_settings:\n  master_key: ''",
    &[],
    (false, false, Some(""))
)]
fn loads_settings(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: (bool, bool, Option<&str>),
) {
    let loaded = settings(source, variables);
    let (disable, allow, signing_key) = expected;
    assert_eq!(loaded.disable_responses_id_security, disable);
    assert_eq!(loaded.allow_unmanaged_response_ids, allow);
    let loaded_key = loaded.signing_key.as_ref().map(ExposeSecret::expose_secret);
    assert_eq!(loaded_key, signing_key);
}

#[test]
fn rejects_a_non_boolean_flag() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  disable_responses_id_security: 'true'")
    .expect_err("a string flag is not a bool");
    assert!(matches!(error, ConfigError::Yaml { .. }), "{error}");
}
