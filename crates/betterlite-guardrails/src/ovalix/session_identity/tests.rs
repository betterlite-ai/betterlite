#![cfg(test)]

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::primitives::json_value::JsonValue;

use super::{RequestData, actor, session_id, tracker_actor_id};

fn values(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, JsonValue> {
    entries
        .iter()
        .map(|(name, value)| {
            let encoded = value.map_or(JsonValue::Null, |text| JsonValue::String(text.to_owned()));
            ((*name).to_owned(), encoded)
        })
        .collect()
}

fn metadata(entries: &[(&str, Option<&str>)]) -> RequestData {
    RequestData {
        metadata: Some(values(entries)),
        litellm_metadata: None,
    }
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

#[test]
fn the_actor_prefers_the_user_email() {
    let data = metadata(&[
        ("user_api_key_user_email", Some("user@example.com")),
        ("user_api_key_user_id", Some("user-123")),
    ]);
    assert_eq!(actor(&data), "user@example.com");
}

#[test]
fn the_actor_falls_back_to_the_user_id() {
    let data = metadata(&[("user_api_key_user_id", Some("user-123"))]);
    assert_eq!(actor(&data), "user-123");
}

#[test]
fn the_actor_is_unknown_without_identity_metadata() {
    assert_eq!(actor(&RequestData::default()), "unknown");
    assert_eq!(actor(&metadata(&[("other", Some("value"))])), "unknown");
}

#[test]
fn the_actor_reads_litellm_metadata_when_metadata_is_empty() {
    let data = RequestData {
        metadata: Some(BTreeMap::new()),
        litellm_metadata: Some(values(&[("user_api_key_user_id", Some("user-123"))])),
    };
    assert_eq!(actor(&data), "user-123");
}

#[test]
fn the_actor_prefers_a_populated_metadata_map() {
    let data = RequestData {
        metadata: Some(values(&[("user_api_key_user_id", Some("metadata-user"))])),
        litellm_metadata: Some(values(&[("user_api_key_user_id", Some("litellm-user"))])),
    };
    assert_eq!(actor(&data), "metadata-user");
}

#[test]
fn the_actor_reads_identity_from_mixed_metadata_values() {
    let data: RequestData = serde_json::from_str(
        r#"{"metadata":{"user_api_key_user_email":"user@example.com","tags":["x"],"user_api_key_spend":0.5,"headers":{"a":"b"}}}"#,
    )
    .expect("request data");
    assert_eq!(actor(&data), "user@example.com");
    assert_eq!(tracker_actor_id(&data), "b4c9a289");
}

#[test]
fn a_blank_identity_value_falls_through_to_the_next_one() {
    let data = metadata(&[
        ("user_api_key_user_email", Some("")),
        ("user_api_key_user_id", Some("user-123")),
    ]);
    assert_eq!(actor(&data), "user-123");
}

#[test]
fn a_null_identity_value_falls_through_to_the_next_one() {
    let data = metadata(&[
        ("user_api_key_user_email", None),
        ("user_api_key_user_id", Some("user-123")),
    ]);
    assert_eq!(actor(&data), "user-123");
}

#[test]
fn the_tracker_actor_id_is_the_first_eight_hex_digits_of_the_sha256() {
    let data = metadata(&[("user_api_key_user_email", Some("user@example.com"))]);
    assert_eq!(tracker_actor_id(&data), "b4c9a289");
    assert_eq!(tracker_actor_id(&RequestData::default()), "b23a6a84");
    assert_eq!(tracker_actor_id(&data).len(), 8);
}

#[test]
fn the_session_id_joins_the_actor_hash_the_date_and_the_application() {
    let data = metadata(&[("user_api_key_user_email", Some("user@example.com"))]);
    assert_eq!(
        session_id(&data, date("2026-03-04"), "app-1"),
        "b4c9a289_2026-03-04_app-1"
    );
}

#[test]
fn the_session_date_is_zero_padded() {
    let data = RequestData::default();
    assert_eq!(
        session_id(&data, date("2026-01-05"), "app-1"),
        "b23a6a84_2026-01-05_app-1"
    );
}
