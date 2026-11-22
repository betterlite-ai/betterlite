#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::openai_realtime::{
    DEFAULT_MAX_MESSAGE_SIZE_BYTES, MAX_MESSAGE_SIZE_VARIABLE, Settings,
};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .openai_realtime)
}

const DEFAULT_SEED: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-backend-transport-default.yaml");
const OVERRIDE_SEED: &str =
    include_str!("../../../fuzz/corpus/config_yaml/realtime-backend-transport-override.yaml");

#[test]
fn loads_realtime_backend_settings() {
    let settings = load(DEFAULT_SEED, &[]).unwrap();
    assert_eq!(
        settings.max_message_size_bytes,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );
    assert_eq!(settings.max_message_size_bytes, 0x0100_0000);

    let numeric = load(OVERRIDE_SEED, &[(MAX_MESSAGE_SIZE_VARIABLE, "4096")]).unwrap();
    assert_eq!(numeric.max_message_size_bytes, 4096);

    let zero = load(DEFAULT_SEED, &[(MAX_MESSAGE_SIZE_VARIABLE, "0")]).unwrap();
    assert_eq!(zero.max_message_size_bytes, 0);

    let empty = load(DEFAULT_SEED, &[(MAX_MESSAGE_SIZE_VARIABLE, "")]).unwrap();
    assert_eq!(empty.max_message_size_bytes, DEFAULT_MAX_MESSAGE_SIZE_BYTES);

    let error = load(DEFAULT_SEED, &[(MAX_MESSAGE_SIZE_VARIABLE, "-1")]).unwrap_err();
    assert!(error.to_string().contains(MAX_MESSAGE_SIZE_VARIABLE));
}

#[rstest]
#[case::negative("-1")]
#[case::text("not-a-number")]
#[case::fractional("1.5")]
fn rejects_values_outside_usize(#[case] rejected: &str) {
    let error = load(DEFAULT_SEED, &[(MAX_MESSAGE_SIZE_VARIABLE, rejected)])
        .expect_err("the cap must reject a value outside usize");
    match error {
        ConfigError::Environment {
            source: betterlite_env::EnvError::Number { name },
        } => assert_eq!(name, MAX_MESSAGE_SIZE_VARIABLE),
        other @ (ConfigError::CustomDimensions(_)
        | ConfigError::Environment { .. }
        | ConfigError::Yaml { .. }
        | ConfigError::Unsupported { .. }
        | ConfigError::MissingEnvironment { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::InvalidInteger { .. }
        | ConfigError::InvalidScalar { .. }
        | ConfigError::LoadThread { .. }) => {
            panic!("unexpected error for {rejected}: {other}")
        }
    }
}
