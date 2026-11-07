#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use super::logger::{BATCH_SIZE_VARIABLE, BatchSettings, FLUSH_INTERVAL_VARIABLE, SettingsError};

#[derive(Debug)]
enum Expected {
    Settings(BatchSettings),
    Error,
}

#[rstest]
#[case::defaults(
    &[],
    Expected::Settings(BatchSettings {
        batch_size: 512,
        flush_interval: Duration::from_secs(5),
        max_queue_size: 50_000,
    })
)]
#[case::environment_overrides_the_defaults(
    &[(BATCH_SIZE_VARIABLE, "7"), (FLUSH_INTERVAL_VARIABLE, "9")],
    Expected::Settings(BatchSettings {
        batch_size: 7,
        flush_interval: Duration::from_secs(9),
        max_queue_size: 50_000,
    })
)]
#[case::blank_variable_takes_the_default(
    &[(BATCH_SIZE_VARIABLE, "  ")],
    Expected::Settings(BatchSettings {
        batch_size: 512,
        flush_interval: Duration::from_secs(5),
        max_queue_size: 50_000,
    })
)]
#[case::invalid_number_is_an_error(&[(BATCH_SIZE_VARIABLE, "many")], Expected::Error)]
#[case::fraction_is_an_error(&[(FLUSH_INTERVAL_VARIABLE, "1.5")], Expected::Error)]
fn batch_delivery_behavior_tests_behavior(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Expected,
) {
    let variables: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let resolved = BatchSettings::from_environment(&variables);
    match (resolved, expected) {
        (Ok(settings), Expected::Settings(wanted)) => assert_eq!(settings, wanted),
        (Err(error), Expected::Error) => assert!(matches!(error, SettingsError::Environment(_))),
        (outcome, expectation) => panic!("unexpected {outcome:?} for {expectation:?}"),
    }
}
