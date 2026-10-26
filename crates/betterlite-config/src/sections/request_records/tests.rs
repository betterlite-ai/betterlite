#![cfg(test)]

use crate::loader::ConfigLoader;
use std::collections::BTreeMap;

#[rstest::rstest]
#[case::loads_default("", None, false)]
#[case::loads_explicit_false("litellm_settings:\n  safe_memory_mode: false\n", Some("true"), false)]
#[case::loads_explicit_true("litellm_settings:\n  safe_memory_mode: true\n", Some("false"), true)]
fn request_records_config_cases(
    #[case] source: &str,
    #[case] environment: Option<&str>,
    #[case] expected: bool,
) {
    let lookup = environment.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([(String::from("LITELLM_SAFE_MEMORY_MODE"), value.to_owned())])
    });
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("the request-record settings are valid");
    assert_eq!(config.sections.request_records.safe_memory_mode, expected);
}
