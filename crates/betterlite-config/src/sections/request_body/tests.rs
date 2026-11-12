#![cfg(test)]

use rstest::rstest;
use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

const VARIABLE: &str = "MAX_REQUEST_BODY_SIZE_TO_REPAIR_MB";
const DEFAULT_BYTES: Option<u64> = Some(1024 * 1024);

fn load(environment: &[(&str, &str)]) -> Option<u64> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load("").unwrap();
    config.sections.request_body.max_repair_bytes
}

#[rstest]
#[case::unset(&[], DEFAULT_BYTES)]
#[case::one_megabyte(&[(VARIABLE, "1")], DEFAULT_BYTES)]
#[case::explicit_override(&[(VARIABLE, "2")], Some(2 * 1024 * 1024))]
#[case::surrounding_space(&[(VARIABLE, " 3 ")], Some(3 * 1024 * 1024))]
#[case::zero_disables_the_cap(&[(VARIABLE, "0")], None)]
#[case::negative_disables_the_cap(&[(VARIABLE, "-1")], None)]
#[case::empty_falls_back(&[(VARIABLE, "")], DEFAULT_BYTES)]
#[case::not_a_number_falls_back(&[(VARIABLE, "many")], DEFAULT_BYTES)]
#[case::fraction_falls_back(&[(VARIABLE, "1.5")], DEFAULT_BYTES)]
#[case::suffix_falls_back(&[(VARIABLE, "2mb")], DEFAULT_BYTES)]
fn loads_settings(#[case] environment: &[(&str, &str)], #[case] expected: Option<u64>) {
    assert_eq!(load(environment), expected);
}

#[test]
fn loads_settings_accepts_the_key_in_a_config_file() {
    let source = "environment_variables:\n  MAX_REQUEST_BODY_SIZE_TO_REPAIR_MB: \"2\"\n";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config
            .environment_variables
            .get(VARIABLE)
            .map(Option::is_some),
        Some(true)
    );
    assert_eq!(config.sections.request_body.max_repair_bytes, DEFAULT_BYTES);
}

#[test]
fn loads_settings_reports_no_unsupported_key() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let result = loader.load("litellm_settings:\n  num_retries: 2\n");
    assert!(!matches!(result, Err(ConfigError::Unsupported { .. })));
}
