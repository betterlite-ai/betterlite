#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{MissingSessionIdPolicy, Settings};

fn load(source: &str) -> Result<Settings, ConfigError> {
    let lookup = BTreeMap::<String, String>::new();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .request_metadata_safety)
}

#[derive(Debug, PartialEq)]
enum Expected {
    Policy(Option<MissingSessionIdPolicy>),
    Invalid,
}

#[rstest]
#[case::unset("", Expected::Policy(None))]
#[case::explicit_null(
    "general_settings:\n  missing_session_id: null\n",
    Expected::Policy(None)
)]
#[case::omit(
    "general_settings:\n  missing_session_id: omit\n",
    Expected::Policy(Some(MissingSessionIdPolicy::Omit))
)]
#[case::generate(
    "general_settings:\n  missing_session_id: generate\n",
    Expected::Policy(Some(MissingSessionIdPolicy::Generate))
)]
#[case::reject(
    "general_settings:\n  missing_session_id: reject\n",
    Expected::Policy(Some(MissingSessionIdPolicy::Reject))
)]
#[case::unknown(
    "general_settings:\n  missing_session_id: pause\n",
    Expected::Policy(Some(MissingSessionIdPolicy::Unrecognized(
        "pause".to_owned()
    )))
)]
#[case::empty_text(
    "general_settings:\n  missing_session_id: \"\"\n",
    Expected::Policy(Some(MissingSessionIdPolicy::Unrecognized(String::new())))
)]
#[case::number("general_settings:\n  missing_session_id: 5\n", Expected::Invalid)]
#[case::boolean("general_settings:\n  missing_session_id: true\n", Expected::Invalid)]
#[case::sequence("general_settings:\n  missing_session_id: [omit]\n", Expected::Invalid)]
#[case::mapping(
    "general_settings:\n  missing_session_id: {policy: omit}\n",
    Expected::Invalid
)]
fn loads_settings(#[case] source: &str, #[case] expected: Expected) {
    match (load(source), expected) {
        (Ok(settings), Expected::Policy(policy)) => {
            assert_eq!(settings.missing_session_id, policy);
        }
        (Err(ConfigError::InvalidScalar { path, .. }), Expected::Invalid) => {
            assert_eq!(path, "general_settings.missing_session_id");
        }
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
}
