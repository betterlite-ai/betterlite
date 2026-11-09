#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use serde_json::json;

use super::{SessionScopeError, get_session_id_from_a2a_params, scope_session_to_principal};
use crate::completion_bridge::wire::{A2AMessage, MessageSendParams, Role};

fn params(extra: &[(&str, AgentJson)]) -> MessageSendParams {
    let mut fields = BTreeMap::new();
    for (name, value) in extra {
        fields.insert((*name).to_owned(), value.clone());
    }
    MessageSendParams {
        message: A2AMessage {
            role: Role("user".to_owned()),
            parts: Vec::new(),
            message_id: None,
            metadata: None,
            extra: fields,
        },
        metadata: None,
        extra: BTreeMap::new(),
    }
}

#[test]
fn the_context_id_is_the_session_id() {
    let request = params(&[("contextId", json!("session-1"))]);

    assert_eq!(
        get_session_id_from_a2a_params(&request),
        Ok(Some("session-1".to_owned()))
    );
}

#[test]
fn a_missing_context_id_is_no_session() {
    let request = params(&[]);

    assert_eq!(get_session_id_from_a2a_params(&request), Ok(None));
}

#[test]
fn a_null_context_id_is_no_session() {
    let request = params(&[("contextId", json!(null))]);

    assert_eq!(get_session_id_from_a2a_params(&request), Ok(None));
}

#[test]
fn a_non_text_context_id_is_an_error() {
    let request = params(&[("contextId", json!(5_i32))]);

    assert_eq!(
        get_session_id_from_a2a_params(&request),
        Err(SessionScopeError::ContextIdType)
    );
}

#[test]
fn the_principal_prefix_is_the_first_16_sha256_hex_chars() {
    let scoped = scope_session_to_principal("session-1", Some("key-2"));

    assert_eq!(scoped, "7c36b0a9dedde119-session-1");
}

#[test]
fn the_principal_is_hashed_from_its_utf8_bytes() {
    let scoped = scope_session_to_principal("session-1", Some("\u{43a}\u{43b}\u{44e}\u{447}"));

    assert_eq!(scoped, "1de36a32af798da0-session-1");
}

#[test]
fn no_principal_keeps_the_session_id() {
    let scoped = scope_session_to_principal("session-1", None);

    assert_eq!(scoped, "session-1");
}

#[test]
fn an_empty_principal_keeps_the_session_id() {
    let scoped = scope_session_to_principal("session-1", Some(""));

    assert_eq!(scoped, "session-1");
}

#[test]
fn principals_scope_the_same_session_id_apart() {
    let first = scope_session_to_principal("session-1", Some("key-2"));
    let second = scope_session_to_principal("session-1", Some("\u{43a}\u{43b}\u{44e}\u{447}"));

    assert_eq!(second, "1de36a32af798da0-session-1");
    assert_ne!(first, second);
}

#[test]
fn a_long_principal_still_yields_16_hex_chars() {
    let principal = "x".repeat(200);

    let scoped = scope_session_to_principal("session-1", Some(&principal));

    let (prefix, session) = scoped.split_once('-').expect("the prefix is separated");
    assert_eq!(session, "session-1");
    assert_eq!(prefix.len(), 16);
    assert!(
        prefix
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
}
