#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::restore_redacted_litellm_params;
use crate::agent_credentials::support::{json, marker, masker, nested};

fn params(entries: &[(&str, AgentJson)]) -> BTreeMap<String, AgentJson> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect()
}

#[test]
fn a_secret_key_echoed_as_the_marker_takes_the_stored_value() {
    let incoming = params(&[("api_key", marker()), ("model", json(r#""gpt""#))]);
    let existing = params(&[("api_key", json(r#""sk-live""#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[
            ("api_key", json(r#""sk-live""#)),
            ("model", json(r#""gpt""#)),
        ])
    );
}

#[test]
fn a_new_secret_value_is_kept_as_sent() {
    let incoming = params(&[("api_key", json(r#""sk-new""#))]);
    let existing = params(&[("api_key", json(r#""sk-live""#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[("api_key", json(r#""sk-new""#))])
    );
}

#[test]
fn an_empty_secret_value_clears_the_stored_credential() {
    let incoming = params(&[("api_key", json(r#""""#))]);
    let existing = params(&[("api_key", json(r#""sk-live""#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[("api_key", json(r#""""#))])
    );
}

#[test]
fn a_secret_left_out_keeps_its_stored_value_and_a_plain_one_is_dropped() {
    let incoming = params(&[("model", json(r#""gpt""#))]);
    let existing = params(&[
        ("api_key", json(r#""sk-live""#)),
        ("temperature", json("0.2")),
    ]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[
            ("api_key", json(r#""sk-live""#)),
            ("model", json(r#""gpt""#)),
        ])
    );
}

#[test]
fn a_secret_key_with_the_marker_and_no_stored_value_is_dropped() {
    let incoming = params(&[("api_key", marker()), ("model", json(r#""gpt""#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &BTreeMap::new(), &masker()),
        params(&[("model", json(r#""gpt""#))])
    );
}

#[test]
fn a_secret_key_is_never_walked() {
    let incoming = params(&[("api_key", json(r#"{"nested":"value"}"#))]);
    let existing = params(&[("api_key", json(r#""sk-live""#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        incoming
    );
}

#[test]
fn a_marker_under_a_plain_key_becomes_null_without_a_stored_value() {
    let incoming = params(&[("config", marker())]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &BTreeMap::new(), &masker()),
        params(&[("config", json("null"))])
    );
}

#[test]
fn a_marker_under_a_plain_key_takes_the_stored_value() {
    let incoming = params(&[("config", marker())]);
    let existing = params(&[("config", json(r#"{"api_key":"sk-live"}"#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        existing
    );
}

#[test]
fn a_stored_marker_is_not_restored_from() {
    let incoming = params(&[("config", marker())]);
    let existing = params(&[("config", marker())]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        incoming
    );
}

#[test]
fn nested_objects_restore_their_secret_keys() {
    let incoming = params(&[(
        "config",
        json(r#"{"api_key":"REDACTED_BY_LITELM","model":"new"}"#),
    )]);
    let existing = params(&[(
        "config",
        json(r#"{"api_key":"sk-live","model":"old","extra":"x"}"#),
    )]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[("config", json(r#"{"api_key":"sk-live","model":"new"}"#),)])
    );
}

#[test]
fn a_whole_list_echoed_as_the_marker_takes_the_stored_list() {
    let incoming = params(&[("providers", marker())]);
    let existing = params(&[("providers", json(r#"[{"api_key":"sk-live"}]"#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        existing
    );
}

#[test]
fn array_items_pair_with_the_stored_array_by_position() {
    let incoming = params(&[(
        "providers",
        json(
            r#"[{"api_key":"REDACTED_BY_LITELM","model":"new"},{"api_key":"sk-second"},{"name":"third"}]"#,
        ),
    )]);
    let existing = params(&[(
        "providers",
        json(r#"[{"api_key":"sk-first","model":"old"}]"#),
    )]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[(
            "providers",
            json(
                r#"[{"api_key":"sk-first","model":"new"},{"api_key":"sk-second"},{"name":"third"}]"#,
            ),
        )])
    );
}

#[test]
fn an_echoed_array_marker_past_the_stored_end_becomes_null() {
    let incoming = params(&[(
        "providers",
        json(r#"["first","REDACTED_BY_LITELM","third"]"#),
    )]);
    let existing = params(&[("providers", json(r#"["stored"]"#))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[("providers", json(r#"["first",null,"third"]"#))])
    );
}

#[test]
fn a_marker_under_the_last_walked_object_still_takes_the_stored_value() {
    let incoming = params(&[("config", nested(10, marker()))]);
    let existing = params(&[("config", nested(10, json(r#""sk-live""#)))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[("config", nested(10, json(r#""sk-live""#)))])
    );
}

#[test]
fn a_marker_past_the_depth_limit_is_left_as_sent() {
    let incoming = params(&[("config", nested(11, marker()))]);
    let existing = params(&[("config", nested(11, json(r#""sk-live""#)))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        incoming
    );
}

#[test]
fn a_secret_object_just_inside_the_depth_limit_is_restored() {
    let secret = json(r#"{"api_key":"REDACTED_BY_LITELM"}"#);
    let stored = json(r#"{"api_key":"sk-live"}"#);
    let incoming = params(&[("config", nested(9, secret))]);
    let existing = params(&[("config", nested(9, stored.clone()))]);
    assert_eq!(
        restore_redacted_litellm_params(&incoming, &existing, &masker()),
        params(&[("config", nested(9, stored))])
    );
}

#[test]
fn a_secret_object_at_the_depth_limit_is_returned_unchanged() {
    let incoming = params(&[(
        "config",
        nested(10, json(r#"{"api_key":"REDACTED_BY_LITELM"}"#)),
    )]);
    let existing = params(&[("config", nested(10, json(r#"{"api_key":"sk-live"}"#)))]);
    let restored = restore_redacted_litellm_params(&incoming, &existing, &masker());
    assert_eq!(restored, incoming);
}
