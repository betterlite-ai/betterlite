#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use super::{DEFAULT_MAX_MESSAGE_SIZE_BYTES, MAX_MESSAGE_SIZE_VARIABLE, Settings};
use crate::loader::{ConfigError, ConfigLoader};

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

#[derive(Clone, Copy)]
enum Case {
    Loads,
    UnrelatedKey,
}

fn assert_max_message_size() {
    assert_eq!(DEFAULT_MAX_MESSAGE_SIZE_BYTES, 16 * 1024 * 1024);
    let defaults = load("", &[]).unwrap();
    assert_eq!(
        defaults.max_message_size_bytes,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );
    let explicit = load("", &[(MAX_MESSAGE_SIZE_VARIABLE, "8388608")]).unwrap();
    assert_eq!(explicit.max_message_size_bytes, 0x0080_0000);
    let padded = load("", &[(MAX_MESSAGE_SIZE_VARIABLE, " 4096 ")]).unwrap();
    assert_eq!(padded.max_message_size_bytes, 4_096);
    let empty = load("", &[(MAX_MESSAGE_SIZE_VARIABLE, "")]).unwrap();
    assert_eq!(empty.max_message_size_bytes, DEFAULT_MAX_MESSAGE_SIZE_BYTES);
    let blank = load("", &[(MAX_MESSAGE_SIZE_VARIABLE, "   ")]).unwrap();
    assert_eq!(blank.max_message_size_bytes, DEFAULT_MAX_MESSAGE_SIZE_BYTES);
    for invalid in ["not-a-number", "1.5", "-5"] {
        assert!(matches!(
            load("", &[(MAX_MESSAGE_SIZE_VARIABLE, invalid)]),
            Err(ConfigError::Environment {
                source: EnvError::Number { name }
            }) if name == MAX_MESSAGE_SIZE_VARIABLE
        ));
    }
}

fn assert_unrelated_key_rejected() {
    let source = "litellm_settings:\n  openai_realtime_unrelated_key: 1\n";
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}

#[rstest]
#[case::loads_realtime_websocket_max_message_size_bytes(Case::Loads)]
#[case::rejects_unrelated_key(Case::UnrelatedKey)]
fn openai_realtime_config_cases(#[case] case: Case) {
    match case {
        Case::Loads => assert_max_message_size(),
        Case::UnrelatedKey => assert_unrelated_key_rejected(),
    }
}
