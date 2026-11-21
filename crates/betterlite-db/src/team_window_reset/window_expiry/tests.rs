#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{WindowExpiryError, window_is_expired};
use crate::budget_permission_rows::JsonData;
use crate::team_window_reset::json_persist::{WindowJsonError, window_fields};
use crate::team_window_reset::tests::support::{naive, window_json};

#[rstest]
#[case::offset("2025-12-30T00:00:00+00:00", true)]
#[case::zulu("2025-12-30T00:00:00Z", true)]
#[case::naive("2025-12-30T00:00:00", true)]
#[case::space("2025-12-30 00:00:00", true)]
#[case::date_only("2025-12-30", true)]
#[case::fraction("2025-12-30T00:00:00.123456", true)]
#[case::no_seconds("2025-12-30T00:00", true)]
#[case::future("2026-01-01T00:00:01+00:00", false)]
#[case::equal("2026-01-01T00:00:00+00:00", true)]
#[case::just_before("2025-12-31T23:59:59.999999+00:00", true)]
fn expiry_compares_the_stored_time_with_now(#[case] text: &str, #[case] expired: bool) {
    assert_eq!(window_is_expired(Some(text), naive(0)).unwrap(), expired);
}

#[test]
fn expiry_without_a_reset_at_never_expires() {
    assert!(!window_is_expired(None, naive(0)).unwrap());
}

#[test]
fn expiry_drops_a_non_utc_offset_instead_of_converting_it() {
    // 02:00 at +02:00 is the same instant as midnight UTC, but LiteLLM keeps the wall time and drops tzinfo.
    assert!(!window_is_expired(Some("2026-01-01T02:00:00+02:00"), naive(0)).unwrap());
    assert!(window_is_expired(Some("2025-12-30T02:00:00+02:00"), naive(0)).unwrap());
}

#[test]
fn expiry_rejects_an_unparsable_reset_at() {
    let error = window_is_expired(Some("yesterday"), naive(0)).unwrap_err();
    assert_eq!(
        error,
        WindowExpiryError::Unparsable {
            text: "yesterday".to_owned()
        }
    );
}

#[test]
fn a_non_string_reset_at_is_a_malformed_window() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "budget_duration".to_owned(),
        JsonData::String("1h".to_owned()),
    );
    fields.insert(
        "reset_at".to_owned(),
        JsonData::Number(serde_json::Number::from(5_u32)),
    );
    let window = JsonData::Object(fields);
    let error = window_fields("k1", &window).unwrap_err();
    assert!(matches!(error, WindowJsonError::ResetAtNotString { .. }));
}

#[test]
fn an_absent_reset_at_is_not_expired() {
    let window = window_json("1h", None, None);
    let fields = window_fields("k1", &window).unwrap();
    assert_eq!(fields.reset_at, None);
    assert!(!window_is_expired(fields.reset_at, naive(0)).unwrap());
}

#[test]
fn a_null_reset_at_is_not_expired() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "budget_duration".to_owned(),
        JsonData::String("1h".to_owned()),
    );
    fields.insert("reset_at".to_owned(), JsonData::Null);
    let window = JsonData::Object(fields);
    let parsed = window_fields("k1", &window).unwrap();
    assert_eq!(parsed.reset_at, None);
}
