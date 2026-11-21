#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::load_realtime_settings;
use crate::loader::ConfigError;
use crate::sections::openai_realtime::{DEFAULT_MAX_MESSAGE_SIZE_BYTES, MAX_MESSAGE_SIZE_VARIABLE};

fn environment(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(MAX_MESSAGE_SIZE_VARIABLE.to_owned(), value.to_owned())])
}

const DEFAULT_SEED: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-backend-transport-default.yaml");
const OVERRIDE_SEED: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-backend-transport-override.yaml");

#[test]
fn loads_realtime_backend_settings() {
    let settings =
        load_realtime_settings(DEFAULT_SEED, &BTreeMap::<String, String>::new()).unwrap();
    assert_eq!(
        settings.max_message_size_bytes,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );
    assert_eq!(settings.max_message_size_bytes, 16_777_216);

    let numeric = load_realtime_settings(OVERRIDE_SEED, &environment("4096")).unwrap();
    assert_eq!(numeric.max_message_size_bytes, 4096);

    let zero = load_realtime_settings(DEFAULT_SEED, &environment("0")).unwrap();
    assert_eq!(zero.max_message_size_bytes, 0);

    let empty = load_realtime_settings(DEFAULT_SEED, &environment("")).unwrap();
    assert_eq!(empty.max_message_size_bytes, DEFAULT_MAX_MESSAGE_SIZE_BYTES);

    let error = load_realtime_settings(DEFAULT_SEED, &environment("-1")).unwrap_err();
    assert!(error.to_string().contains(MAX_MESSAGE_SIZE_VARIABLE));
}

#[rstest]
#[case::negative("-1")]
#[case::text("not-a-number")]
#[case::fractional("1.5")]
fn rejects_values_outside_usize(#[case] rejected: &str) {
    let error = load_realtime_settings(DEFAULT_SEED, &environment(rejected))
        .expect_err("the cap must reject a value outside usize");
    match error {
        ConfigError::Environment {
            source: betterlite_env::EnvError::Number { name },
        } => assert_eq!(name, MAX_MESSAGE_SIZE_VARIABLE),
        other => panic!("unexpected error for {rejected}: {other}"),
    }
}
