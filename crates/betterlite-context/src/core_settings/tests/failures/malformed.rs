#![cfg(test)]
use super::{BTreeMap, ConfigLoader};
use crate::core_settings::CoreSettings;
use rstest::rstest;

#[rstest]
#[case::malformed("nope")]
#[case::fraction_not_number("1/2")]
fn call_context_tests_behavior(#[case] raw: &str) {
    let env = BTreeMap::from([("REQUEST_TIMEOUT".to_owned(), raw.to_owned())]);
    let loader = ConfigLoader { lookup: env };
    let config = loader.load("{}").unwrap();
    let error = CoreSettings::load(&config, &loader.lookup).unwrap_err();
    assert!(error.to_string().contains("REQUEST_TIMEOUT"));
}
