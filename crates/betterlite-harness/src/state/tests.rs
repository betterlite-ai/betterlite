#![cfg(test)]
use rstest::rstest;

use crate::errors::HarnessError;
use crate::state::{STATE_VERSION, State};
use crate::types::Harness;

#[test]
fn dumps_in_litellm_field_order_and_loads_back() {
    let state = State {
        harness: Harness::Codex,
        native_session_id: Some("thread-1".to_owned()),
        workdir: "/work".to_owned(),
        model: Some("gpt-5".to_owned()),
        version: STATE_VERSION,
    };
    let bytes = serde_json::to_vec(&state).unwrap();
    assert_eq!(
        bytes,
        br#"{"harness":"codex","native_session_id":"thread-1","workdir":"/work","model":"gpt-5","version":1}"#
    );
    assert_eq!(State::loads(&bytes).unwrap(), state);
}

#[test]
fn loads_ignores_unknown_fields_and_defaults_optional_ones() {
    let state =
        State::loads(br#"{"harness":"opencode","workdir":"/w","version":1,"extra":true}"#).unwrap();
    let expected = State {
        harness: Harness::OpenCode,
        native_session_id: None,
        workdir: "/w".to_owned(),
        model: None,
        version: 1,
    };
    assert_eq!(state, expected);
}

#[rstest]
#[case::missing_harness(br#"{"workdir":"/w","version":1}"#.as_slice())]
#[case::unknown_harness(br#"{"harness":"CODEX","workdir":"/w","version":1}"#.as_slice())]
#[case::missing_workdir(br#"{"harness":"codex","version":1}"#.as_slice())]
#[case::missing_version(br#"{"harness":"codex","workdir":"/w"}"#.as_slice())]
#[case::not_an_object(b"[1]".as_slice())]
#[case::invalid_utf8(b"{\"harness\":\"codex\xff\",\"workdir\":\"/w\",\"version\":1}".as_slice())]
fn loads_rejects_unreadable_state(#[case] data: &[u8]) {
    assert!(matches!(
        State::loads(data),
        Err(HarnessError::StateIncompatible { .. })
    ));
}

#[test]
fn loads_rejects_other_versions() {
    let loaded = State::loads(br#"{"harness":"codex","workdir":"/w","version":2}"#);
    assert!(matches!(
        loaded,
        Err(HarnessError::StateVersion { found: 2 })
    ));
}
