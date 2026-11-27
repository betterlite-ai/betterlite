#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::Expiry;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use chrono::{DateTime, TimeDelta, Utc};
use rstest::rstest;

use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::{active_key_user_id, key_is_active, user_scim_deactivated};

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_780_000_000, 0).unwrap()
}

fn key_row(fields: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).unwrap()
}

fn shifted(offset_seconds: i64) -> DateTime<Utc> {
    let shift = TimeDelta::try_seconds(offset_seconds).unwrap();
    now().checked_add_signed(shift).unwrap()
}

#[rstest]
#[case::unblocked_without_expiry(r#"{"user_id":"user-1"}"#, true)]
#[case::blocked(r#"{"user_id":"user-1","blocked":true}"#, false)]
#[case::explicitly_unblocked(r#"{"user_id":"user-1","blocked":false}"#, true)]
#[case::future_text_expiry(r#"{"user_id":"user-1","expires":"2099-01-01T00:00:00Z"}"#, true)]
#[case::past_text_expiry(r#"{"user_id":"user-1","expires":"2020-01-01T00:00:00Z"}"#, false)]
#[case::naive_future_text_expiry(r#"{"user_id":"user-1","expires":"2099-01-01T00:00:00"}"#, true)]
#[case::naive_past_text_expiry(r#"{"user_id":"user-1","expires":"2020-01-01T00:00:00"}"#, false)]
#[case::offset_future_expiry(r#"{"user_id":"user-1","expires":"2099-01-01T00:00:00+02:00"}"#, true)]
#[case::offset_past_expiry(r#"{"user_id":"user-1","expires":"2020-01-01T00:00:00-05:00"}"#, false)]
#[case::unparseable_expiry(r#"{"user_id":"user-1","expires":"not-a-timestamp"}"#, false)]
#[case::team_key_without_user_id(r#"{"expires":"2099-01-01T00:00:00Z"}"#, true)]
fn key_is_active_reads_blocked_and_expiry(#[case] fields: &str, #[case] expected: bool) {
    assert_eq!(key_is_active(&key_row(fields), now()), expected);
}

#[rstest]
#[case::at_now(0_i64, true)]
#[case::one_second_past(-1_i64, false)]
#[case::one_second_ahead(1_i64, true)]
fn key_is_active_compares_a_datetime_expiry_with_the_instant(
    #[case] offset_seconds: i64,
    #[case] expected: bool,
) {
    let mut auth = key_row(r#"{"user_id":"user-1"}"#);
    auth.view.token.expires = Some(Expiry::DateTime(shifted(offset_seconds).fixed_offset()));
    assert_eq!(key_is_active(&auth, now()), expected);
}

#[rstest]
#[case::at_now(0_i64, true)]
#[case::one_second_past(-1_i64, false)]
fn key_is_active_compares_a_text_expiry_with_the_instant(
    #[case] offset_seconds: i64,
    #[case] expected: bool,
) {
    let fields = format!(
        r#"{{"user_id":"user-1","expires":"{}"}}"#,
        shifted(offset_seconds).to_rfc3339()
    );
    assert_eq!(key_is_active(&key_row(&fields), now()), expected);
}

#[rstest]
#[case::active_key(r#"{"user_id":"user-1"}"#, Some("user-1"))]
#[case::blocked_key(r#"{"user_id":"user-1","blocked":true}"#, None)]
#[case::expired_key(r#"{"user_id":"user-1","expires":"2020-01-01T00:00:00Z"}"#, None)]
#[case::team_key(r#"{"expires":"2099-01-01T00:00:00Z"}"#, None)]
fn key_active_user_id_needs_a_live_key_with_an_owner(
    #[case] fields: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(active_key_user_id(&key_row(fields), now()), expected);
}

fn user_row(metadata: &str) -> LiteLLMUserTable {
    let row = format!(r#"{{"user_id":"user-1","metadata":{metadata}}}"#);
    serde_json::from_str(&row).unwrap()
}

#[rstest]
#[case::scim_false(r#"{"scim_active":false}"#, true)]
#[case::scim_true(r#"{"scim_active":true}"#, false)]
#[case::scim_null(r#"{"scim_active":null}"#, false)]
#[case::scim_zero(r#"{"scim_active":0}"#, false)]
#[case::scim_string_false(r#"{"scim_active":"false"}"#, false)]
#[case::missing_entry(r#"{"other":false}"#, false)]
#[case::empty_metadata("{}", false)]
fn user_scim_deactivated_needs_a_literal_false(#[case] metadata: &str, #[case] expected: bool) {
    assert_eq!(user_scim_deactivated(&user_row(metadata)), expected);
}

#[test]
fn user_scim_deactivated_is_false_without_metadata() {
    let row: LiteLLMUserTable = serde_json::from_str(r#"{"user_id":"user-1"}"#).unwrap();
    assert!(!user_scim_deactivated(&row));
}
