#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use rstest::rstest;

use super::{Borrowed, load_realtime_settings};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::openai_realtime::{DEFAULT_MAX_MESSAGE_SIZE_BYTES, MAX_MESSAGE_SIZE_VARIABLE};

fn environment(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(MAX_MESSAGE_SIZE_VARIABLE.to_owned(), value.to_owned())])
}

// an environment whose only extra is the home lookup the loader forwards
struct Home(PathBuf);

impl EnvironmentLookup for Home {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }

    fn system_home(&self) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

#[test]
fn forwards_the_system_home() {
    let home = Home(PathBuf::from("/opt/litellm"));
    assert_eq!(
        Borrowed(&home).system_home(),
        Some(PathBuf::from("/opt/litellm"))
    );
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
    assert_eq!(settings.max_message_size_bytes, 0x0100_0000);

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
        other @ ConfigError::CustomDimensions(_)
        | other @ ConfigError::Environment { .. }
        | other @ ConfigError::Yaml { .. }
        | other @ ConfigError::Unsupported { .. }
        | other @ ConfigError::MissingEnvironment { .. }
        | other @ ConfigError::InvalidFloat { .. }
        | other @ ConfigError::InvalidInteger { .. }
        | other @ ConfigError::InvalidScalar { .. }
        | other @ ConfigError::LoadThread { .. } => {
            panic!("unexpected error for {rejected}: {other}")
        }
    }
}
