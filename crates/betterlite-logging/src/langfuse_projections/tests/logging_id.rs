#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Timelike as _, Utc};
use indexmap::IndexMap;

use super::ids::{logging_id, lookup_ids};
use super::support::value;

fn start_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 1, 2, 3)
        .single()
        .expect("a valid date")
        .with_nanosecond(456_789_000)
        .expect("a valid nanosecond")
}

#[test]
fn logging_id_formats_the_start_time_and_response_id() {
    let response = value(r#"{"id": "chatcmpl-1"}"#);
    assert_eq!(
        logging_id(Some(&start_time()), Some(&response)),
        Some("time-01-02-03-456789_chatcmpl-1".to_owned())
    );
}

#[test]
fn logging_id_is_none_without_a_start_time_or_a_text_id() {
    let response = value(r#"{"id": "chatcmpl-1"}"#);
    assert_eq!(logging_id(None, Some(&response)), None);
    assert_eq!(
        logging_id(Some(&start_time()), Some(&value(r#"{"id": 5}"#))),
        None
    );
    assert_eq!(
        logging_id(Some(&start_time()), Some(&value(r#"{"object": "chat"}"#))),
        None
    );
}

#[test]
fn lookup_ids_returns_the_call_id_and_response_id() {
    let response = value(r#"{"id": "chatcmpl-1"}"#);
    let ids = lookup_ids(Some("call-1"), Some(&response));
    let expected: IndexMap<String, String> = [
        ("litellm_call_id".to_owned(), "call-1".to_owned()),
        ("response_id".to_owned(), "chatcmpl-1".to_owned()),
    ]
    .into_iter()
    .collect();
    assert_eq!(ids, expected);
}

#[test]
fn response_id_is_absent_without_an_id() {
    assert_eq!(
        lookup_ids(None, Some(&value(r#"{"object": "chat"}"#))).len(),
        0
    );
    assert_eq!(lookup_ids(None, Some(&value(r#"{"id": null}"#))).len(), 0);
    assert_eq!(
        lookup_ids(None, Some(&Json::Text("not a response".to_owned()))).len(),
        0
    );
}

#[test]
fn response_id_is_rendered_from_a_number() {
    let ids = lookup_ids(None, Some(&value(r#"{"id": 42}"#)));
    assert_eq!(ids.get("response_id"), Some(&"42".to_owned()));
}
