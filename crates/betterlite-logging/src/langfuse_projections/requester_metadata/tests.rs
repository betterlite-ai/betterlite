#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::support::value;
use super::log_requester_metadata;

fn metadata(text: &str) -> IndexMap<String, Json> {
    match value(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is an object")
        }
    }
}

#[test]
fn requester_metadata_is_hoisted_and_kept_flat() {
    let input = metadata(r#"{"user": "max", "requester_metadata": {"requester_ip": "127.0.0.1"}}"#);
    assert_eq!(
        log_requester_metadata(&input),
        metadata(r#"{"user": "max", "requester_metadata": {"requester_ip": "127.0.0.1"}}"#)
    );
}

#[test]
fn requester_metadata_wins_over_a_top_level_duplicate() {
    let input = metadata(
        r#"{"requester_ip": "top-level", "requester_metadata": {"requester_ip": "nested"}}"#,
    );
    let expected = metadata(r#"{"requester_metadata": {"requester_ip": "nested"}}"#);
    assert_eq!(log_requester_metadata(&input), expected);
}

#[test]
fn requester_metadata_defaults_to_an_empty_object() {
    let input = metadata(r#"{"user": "max"}"#);
    assert_eq!(
        log_requester_metadata(&input),
        metadata(r#"{"user": "max", "requester_metadata": {}}"#)
    );
    let null = metadata(r#"{"user": "max", "requester_metadata": null}"#);
    assert_eq!(
        log_requester_metadata(&null),
        metadata(r#"{"user": "max", "requester_metadata": {}}"#)
    );
}
