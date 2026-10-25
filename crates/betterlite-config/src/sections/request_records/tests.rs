#![cfg(test)]

use crate::loader::ConfigLoader;
use std::collections::BTreeMap;

#[rstest::rstest]
#[case::loads_safe_memory_mode("litellm_settings:\n  safe_memory_mode: true\n", true)]
fn request_records_config_cases(#[case] source: &str, #[case] expected: bool) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the request-record settings are valid");
    assert_eq!(config.sections.request_records.safe_memory_mode, expected);
}
