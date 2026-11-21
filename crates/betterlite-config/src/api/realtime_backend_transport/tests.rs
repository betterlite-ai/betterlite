use std::collections::BTreeMap;

use super::loads_realtime_backend_settings;
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
        loads_realtime_backend_settings(DEFAULT_SEED, &BTreeMap::<String, String>::new()).unwrap();
    assert_eq!(
        settings.max_message_size_bytes,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );
    assert_eq!(settings.max_message_size_bytes, 16_777_216);

    let numeric = loads_realtime_backend_settings(OVERRIDE_SEED, &environment("4096")).unwrap();
    assert_eq!(numeric.max_message_size_bytes, 4096);

    let zero = loads_realtime_backend_settings(DEFAULT_SEED, &environment("0")).unwrap();
    assert_eq!(zero.max_message_size_bytes, 0);

    let empty = loads_realtime_backend_settings(DEFAULT_SEED, &environment("")).unwrap();
    assert_eq!(empty.max_message_size_bytes, DEFAULT_MAX_MESSAGE_SIZE_BYTES);

    for rejected in ["-1", "not-a-number", "1.5"] {
        let error = loads_realtime_backend_settings(DEFAULT_SEED, &environment(rejected))
            .expect_err("the cap must reject a value outside usize");
        match error {
            ConfigError::Environment {
                source: betterlite_env::EnvError::Number { name },
            } => assert_eq!(name, MAX_MESSAGE_SIZE_VARIABLE),
            other => panic!("unexpected error for {rejected}: {other}"),
        }
    }

    let error = loads_realtime_backend_settings(DEFAULT_SEED, &environment("-1")).unwrap_err();
    assert!(error.to_string().contains(MAX_MESSAGE_SIZE_VARIABLE));
}
