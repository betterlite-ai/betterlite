#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::routing_state::Settings;

fn settings(variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []")
        .map(|config| config.sections.routing_state)
        .map_err(|error| error.to_string())
}

#[rstest]
#[case::absent_defaults(&[], Duration::from_secs(1))]
#[case::configured_seconds(&[("DEFAULT_REDIS_SYNC_INTERVAL", "7")], Duration::from_secs(7))]
#[case::zero_is_used_as_given(&[("DEFAULT_REDIS_SYNC_INTERVAL", "0")], Duration::ZERO)]
#[case::empty_environment_is_unset(&[("DEFAULT_REDIS_SYNC_INTERVAL", "")], Duration::from_secs(1))]
fn converts_the_sync_interval(#[case] variables: &[(&str, &str)], #[case] expected: Duration) {
    let converted = settings(variables).unwrap();
    assert_eq!(converted.default_sync_interval, expected);
}

#[rstest]
#[case::text("soon")]
#[case::negative("-1")]
#[case::fraction("0.5")]
fn rejects_a_sync_interval_that_is_not_whole_seconds(#[case] value: &str) {
    let error = settings(&[("DEFAULT_REDIS_SYNC_INTERVAL", value)]).unwrap_err();
    assert!(error.contains("DEFAULT_REDIS_SYNC_INTERVAL"), "{error}");
}
