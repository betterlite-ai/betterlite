#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, FixedOffset, Utc};
use rstest::rstest;

use crate::budget_primitives::carried::{
    carried_metadata, carry_organization, carry_team_and_user,
};
use crate::budget_primitives::throttle::{percentage, should_throttle, throttled_limit};
use crate::budget_primitives::window::{
    BudgetWindow, BudgetWindowModel, MAX_WINDOW_BYTES, MAX_WINDOW_DEPTH, coerce_model,
    coerce_window, decode_budget_window,
};
use crate::budget_primitives::window_start::{
    DurationParser, budget_window_start, coerce_datetime,
};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

fn token(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(fields).unwrap() else {
        panic!("token fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn window(text: &str) -> BudgetWindow {
    decode_budget_window(text.as_bytes())
}

fn instant(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn team(text: &str) -> LiteLLMTeamTable {
    serde_json::from_str(text).unwrap()
}

fn user(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).unwrap()
}

fn organization(text: &str) -> OrganizationTable {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::absent(None, None)]
#[case::fraction(Some(0.5_f64), Some(0.5_f64))]
#[case::one(Some(1.0_f64), Some(1.0_f64))]
#[case::zero(Some(0.0_f64), None)]
#[case::negative(Some(-0.5_f64), None)]
#[case::above_one(Some(1.5_f64), None)]
#[case::nan(Some(f64::NAN), None)]
#[case::infinity(Some(f64::INFINITY), None)]
#[case::negative_infinity(Some(f64::NEG_INFINITY), None)]
fn throttle_percentage_type_range_and_nonfinite(
    #[case] configured: Option<f64>,
    #[case] expected: Option<f64>,
) {
    assert_eq!(percentage(configured), expected);
}

#[rstest]
#[case::opted_in_with_tpm(
    r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#,
    Some(0.5_f64),
    true
)]
#[case::opted_in_with_rpm(
    r#"{"metadata":{"throttle_on_budget_exceeded":true},"rpm_limit":10}"#,
    Some(1.0_f64),
    true
)]
#[case::opted_in_with_zero_limit(
    r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":0}"#,
    Some(0.5_f64),
    true
)]
#[case::opted_in_with_negative_limit(
    r#"{"metadata":{"throttle_on_budget_exceeded":true},"rpm_limit":-5}"#,
    Some(0.5_f64),
    true
)]
#[case::string_opt_in_is_not_true(
    r#"{"metadata":{"throttle_on_budget_exceeded":"true"},"tpm_limit":10}"#,
    Some(0.5_f64),
    false
)]
#[case::numeric_opt_in_is_not_true(
    r#"{"metadata":{"throttle_on_budget_exceeded":1},"tpm_limit":10}"#,
    Some(0.5_f64),
    false
)]
#[case::missing_metadata("{}", Some(0.5_f64), false)]
#[case::no_limit(
    r#"{"metadata":{"throttle_on_budget_exceeded":true}}"#,
    Some(0.5_f64),
    false
)]
#[case::disabled_setting(
    r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#,
    None,
    false
)]
#[case::invalid_setting(
    r#"{"metadata":{"throttle_on_budget_exceeded":true},"tpm_limit":10}"#,
    Some(2.0_f64),
    false
)]
fn exact_opt_in_and_nonnull_limit(
    #[case] fields: &str,
    #[case] configured: Option<f64>,
    #[case] expected: bool,
) {
    assert_eq!(should_throttle(&token(fields), configured), expected);
}

#[rstest]
#[case::unset_limit(None, Some(0.5_f64), None)]
#[case::unset_setting(Some(100), None, Some(100))]
#[case::scaled(Some(100), Some(0.5_f64), Some(50))]
#[case::one(Some(3), Some(1.0_f64), Some(3))]
#[case::zero_limit_keeps_a_trickle(Some(0), Some(0.5_f64), Some(1))]
#[case::negative_limit_keeps_a_trickle(Some(-5), Some(0.5_f64), Some(1))]
#[case::floor_of_fraction_is_one(Some(1), Some(0.1_f64), Some(1))]
#[case::float_error_floors_down(Some(100), Some(0.29_f64), Some(28))]
#[case::both_unset(None, None, None)]
fn throttle_floor_minimum_one_and_unset(
    #[case] limit: Option<i64>,
    #[case] configured: Option<f64>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(throttled_limit(limit, configured), expected);
}

struct Dump(JsonData);

struct Raising;

#[derive(Debug, thiserror::Error)]
#[error("model dump failed")]
struct DumpError;

impl BudgetWindowModel for Dump {
    type Error = DumpError;

    fn model_dump(&self) -> Result<JsonData, Self::Error> {
        Ok(self.0.clone())
    }
}

impl BudgetWindowModel for Raising {
    type Error = DumpError;

    fn model_dump(&self) -> Result<JsonData, Self::Error> {
        Err(DumpError)
    }
}

#[test]
fn window_mapping_json_scalar_dump_and_raising_dump() {
    let mapping = json(r#"{"budget_duration":"1h"}"#);
    assert_eq!(
        coerce_window(&mapping).as_ref()["budget_duration"],
        JsonData::String("1h".to_owned())
    );
    let text = json(r#""{\"budget_duration\":\"2h\"}""#);
    assert_eq!(
        coerce_window(&text).as_ref()["budget_duration"],
        JsonData::String("2h".to_owned())
    );
    let scalar_text = json(r#""42""#);
    assert!(coerce_window(&scalar_text).as_ref().is_empty());
    let malformed = json(r#""not json""#);
    assert!(coerce_window(&malformed).as_ref().is_empty());
    assert!(coerce_window(&json("5")).as_ref().is_empty());
    assert!(coerce_window(&json("true")).as_ref().is_empty());
    assert!(coerce_window(&json("[1]")).as_ref().is_empty());
    assert!(coerce_window(&json("null")).as_ref().is_empty());
    let dumped = coerce_model(&Dump(json(r#"{"budget_duration":"3h"}"#))).unwrap();
    assert_eq!(
        dumped.as_ref()["budget_duration"],
        JsonData::String("3h".to_owned())
    );
    assert!(
        coerce_model(&Dump(json("true")))
            .unwrap()
            .as_ref()
            .is_empty()
    );
    coerce_model(&Raising).unwrap_err();
}

struct Hourly;

impl DurationParser for Hourly {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1h" => Some(3_600),
            "2h" => Some(7_200),
            _ => None,
        }
    }
}

#[rstest]
#[case::valid(
    r#"{"budget_duration":"1h","reset_at":"2026-06-01T00:00:00Z"}"#,
    Some("2026-05-31T23:00:00+00:00")
)]
#[case::other_unit(
    r#"{"budget_duration":"2h","reset_at":"2026-06-01T00:00:00Z"}"#,
    Some("2026-05-31T22:00:00+00:00")
)]
#[case::invalid_duration(
    r#"{"budget_duration":"bogus","reset_at":"2026-06-01T00:00:00Z"}"#,
    None
)]
#[case::numeric_duration(r#"{"budget_duration":5,"reset_at":"2026-06-01T00:00:00Z"}"#, None)]
#[case::null_duration(r#"{"budget_duration":null,"reset_at":"2026-06-01T00:00:00Z"}"#, None)]
#[case::absent_duration(r#"{"reset_at":"2026-06-01T00:00:00Z"}"#, None)]
fn duration_absent_invalid_and_string_coercion(
    #[case] fields: &str,
    #[case] expected: Option<&str>,
) {
    let now = instant("2026-06-01T12:00:00Z").with_timezone(&Utc);
    let start = budget_window_start(&window(fields), now, &Hourly);
    assert_eq!(start, expected.map(instant));
}

#[rstest]
#[case::z(
    r#"{"budget_duration":"1h","reset_at":"2026-06-01T00:00:00Z"}"#,
    "2026-05-31T23:00:00+00:00",
    0_i32
)]
#[case::offset(
    r#"{"budget_duration":"1h","reset_at":"2026-06-01T00:00:00+05:30"}"#,
    "2026-05-31T23:00:00+05:30",
    19_800_i32
)]
#[case::naive(
    r#"{"budget_duration":"1h","reset_at":"2026-06-01T00:00:00"}"#,
    "2026-05-31T23:00:00+00:00",
    0_i32
)]
#[case::date_only(
    r#"{"budget_duration":"1h","reset_at":"2026-06-01"}"#,
    "2026-05-31T23:00:00+00:00",
    0_i32
)]
#[case::fraction(
    r#"{"budget_duration":"1h","reset_at":"2026-06-01T00:00:00.500Z"}"#,
    "2026-05-31T23:00:00.500+00:00",
    0_i32
)]
fn reset_datetime_z_naive_offset_invalid(
    #[case] fields: &str,
    #[case] expected: &str,
    #[case] offset_seconds: i32,
) {
    let now = instant("2026-06-01T12:00:00Z").with_timezone(&Utc);
    let start = budget_window_start(&window(fields), now, &Hourly).unwrap();
    assert_eq!(start, instant(expected));
    assert_eq!(start.offset().local_minus_utc(), offset_seconds);
}

#[rstest]
#[case::malformed_date(r#"{"budget_duration":"1h","reset_at":"not-a-date"}"#)]
#[case::number_reset(r#"{"budget_duration":"1h","reset_at":1700000000}"#)]
#[case::boolean_reset(r#"{"budget_duration":"1h","reset_at":true}"#)]
#[case::object_reset(r#"{"budget_duration":"1h","reset_at":{"at":"now"}}"#)]
fn missing_reset_uses_injected_now(#[case] fields: &str) {
    let now = instant("2026-06-01T12:00:00Z").with_timezone(&Utc);
    assert_eq!(
        budget_window_start(&window(fields), now, &Hourly),
        Some(instant("2026-06-01T11:00:00+00:00"))
    );
    let absent = r#"{"budget_duration":"1h"}"#;
    assert_eq!(
        budget_window_start(&window(absent), now, &Hourly),
        Some(instant("2026-06-01T11:00:00+00:00"))
    );
}

#[test]
fn missing_reset_does_not_read_the_wall_clock() {
    let now = instant("1999-12-31T23:30:00Z").with_timezone(&Utc);
    assert_eq!(
        budget_window_start(&window(r#"{"budget_duration":"1h"}"#), now, &Hourly),
        Some(instant("1999-12-31T22:30:00+00:00"))
    );
}

#[test]
fn coerce_datetime_retains_and_rejects_shapes() {
    assert_eq!(
        coerce_datetime(Some(&json(r#""2026-06-01T00:00:00+02:00""#))),
        Some(instant("2026-06-01T00:00:00+02:00"))
    );
    assert_eq!(
        coerce_datetime(Some(&json(r#""2026-06-01T00:00:00""#))),
        Some(instant("2026-06-01T00:00:00+00:00"))
    );
    // fromisoformat keeps six fraction digits; the truncating parser must do the same
    assert_eq!(
        coerce_datetime(Some(&json(r#""2026-06-01T00:00:00.123456789Z""#))),
        Some(instant("2026-06-01T00:00:00.123456+00:00"))
    );
    assert_eq!(coerce_datetime(Some(&json("null"))), None);
    assert_eq!(coerce_datetime(Some(&json("12"))), None);
    assert_eq!(coerce_datetime(Some(&json(r#"["x"]"#))), None);
    assert_eq!(coerce_datetime(None), None);
}

#[test]
fn snapshots_assign_only_present_objects() {
    let mut token = token(
        r#"{"team_budget_snapshot":{"budget_reset_at":"2026-01-01T00:00:00Z","max_budget":7.5}}"#,
    );
    carry_team_and_user(&mut token, None, None);
    let kept = carried_metadata(&token).unwrap();
    assert_eq!(
        kept["user_api_key_team_budget_reset_at"].get(),
        r#""2026-01-01T00:00:00Z""#
    );
    assert_eq!(kept["user_api_key_team_table_max_budget"].get(), "7.5");
    let user_only = user(r#"{"user_id":"v"}"#);
    carry_team_and_user(&mut token, None, Some(&user_only));
    let latest = carried_metadata(&token).unwrap();
    assert_eq!(latest["user_api_key_user_budget_reset_at"].get(), "null");
    assert_eq!(latest["user_api_key_user_table_max_budget"].get(), "null");
    assert_eq!(latest["user_api_key_user_alias"].get(), "null");
    assert_eq!(latest["user_api_key_team_table_max_budget"].get(), "7.5");
    assert_eq!(
        latest["user_api_key_team_budget_reset_at"].get(),
        r#""2026-01-01T00:00:00Z""#
    );
}

#[test]
fn present_rows_replace_snapshots() {
    let mut token = token("{}");
    let team_row = team(
        r#"{"team_id":"t","budget_reset_at":"2026-02-01T00:00:00+02:00","max_budget":12.5,"model_max_budget":{"gpt-4":1.5}}"#,
    );
    let user_row =
        user(r#"{"user_id":"u","user_alias":"ali","budget_reset_at":null,"max_budget":3.5}"#);
    carry_team_and_user(&mut token, Some(&team_row), Some(&user_row));
    let carried = carried_metadata(&token).unwrap();
    assert_eq!(
        carried["user_api_key_team_budget_reset_at"].get(),
        r#""2026-02-01T00:00:00+02:00""#
    );
    assert_eq!(carried["user_api_key_team_table_max_budget"].get(), "12.5");
    assert_eq!(carried["user_api_key_user_budget_reset_at"].get(), "null");
    assert_eq!(carried["user_api_key_user_table_max_budget"].get(), "3.5");
    assert_eq!(carried["user_api_key_user_alias"].get(), r#""ali""#);
    assert_eq!(
        serde_json::to_string(&token.view.team_model_max_budget).unwrap(),
        r#"{"gpt-4":1.5}"#
    );
}

#[test]
fn organization_missing_budget_still_carries_alias_spend() {
    let mut token = token("{}");
    let org = organization(
        r#"{"organization_id":"o","organization_alias":"acme","budget_id":"b","spend":12.5,"created_by":"c","updated_by":"u"}"#,
    );
    carry_organization(&mut token, &org);
    assert_eq!(token.view.organization_alias.as_deref(), Some("acme"));
    let metadata = carried_metadata(&token).unwrap();
    assert_eq!(metadata["user_api_key_org_spend"].get(), "12.5");
    assert_eq!(metadata["user_api_key_org_max_budget"].get(), "null");
    let with_budget = organization(
        r#"{"organization_id":"o","organization_alias":"acme","budget_id":"b","spend":1.5,"created_by":"c","updated_by":"u","litellm_budget_table":{"max_budget":9.5}}"#,
    );
    carry_organization(&mut token, &with_budget);
    let carried = carried_metadata(&token).unwrap();
    assert_eq!(carried["user_api_key_org_spend"].get(), "1.5");
    assert_eq!(carried["user_api_key_org_max_budget"].get(), "9.5");
}

#[test]
fn metadata_snapshot_order_and_immutability() {
    let mut token = token("{}");
    let team_row = team(r#"{"team_id":"t","max_budget":1.5}"#);
    let user_row = user(r#"{"user_id":"u","user_alias":"ali","max_budget":2.5}"#);
    let org_row = organization(
        r#"{"organization_id":"o","organization_alias":"acme","budget_id":"b","spend":3.5,"created_by":"c","updated_by":"u","litellm_budget_table":{"max_budget":4.5}}"#,
    );
    carry_team_and_user(&mut token, Some(&team_row), Some(&user_row));
    carry_organization(&mut token, &org_row);
    let first = carried_metadata(&token).unwrap();
    assert_eq!(first.len(), 7);
    assert_eq!(first["user_api_key_team_table_max_budget"].get(), "1.5");
    assert_eq!(first["user_api_key_user_table_max_budget"].get(), "2.5");
    assert_eq!(first["user_api_key_org_spend"].get(), "3.5");
    let later = organization(
        r#"{"organization_id":"o","organization_alias":"other","budget_id":"b","spend":8.5,"created_by":"c","updated_by":"u"}"#,
    );
    carry_organization(&mut token, &later);
    assert_eq!(first["user_api_key_org_spend"].get(), "3.5");
    let second = carried_metadata(&token).unwrap();
    assert_eq!(second["user_api_key_org_spend"].get(), "8.5");
    assert_eq!(second["user_api_key_org_max_budget"].get(), "null");
    assert_eq!(token.view.organization_alias.as_deref(), Some("other"));
}

#[rstest]
#[case::utc("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z")]
#[case::offset("2026-01-01T00:00:00+05:30", "2026-01-01T00:00:00+05:30")]
#[case::negative_offset("2026-01-01T00:00:00-05:00", "2026-01-01T00:00:00-05:00")]
#[case::naive("2026-01-01T00:00:00", "2026-01-01T00:00:00")]
#[case::fraction("2026-01-01T00:00:00.4Z", "2026-01-01T00:00:00.400000Z")]
fn snapshot_datetimes_use_pydantic_text(#[case] reset_at: &str, #[case] expected: &str) {
    let mut token = token("{}");
    let team_row = team(&format!(
        r#"{{"team_id":"t","budget_reset_at":"{reset_at}"}}"#
    ));
    carry_team_and_user(&mut token, Some(&team_row), None);
    let metadata = carried_metadata(&token).unwrap();
    assert_eq!(
        metadata["user_api_key_team_budget_reset_at"].get(),
        format!(r#""{expected}""#)
    );
}

fn padded_window(target_bytes: usize) -> String {
    let header = r#"{"a":1}"#;
    format!(
        "{header}{}",
        " ".repeat(target_bytes.saturating_sub(header.len()))
    )
}

#[test]
fn native_window_limits_return_empty_mapping() {
    let at_limit = padded_window(MAX_WINDOW_BYTES);
    assert_eq!(at_limit.len(), MAX_WINDOW_BYTES);
    assert_eq!(decode_budget_window(at_limit.as_bytes()).as_ref().len(), 1);
    let oversize = padded_window(MAX_WINDOW_BYTES.saturating_add(1));
    assert_eq!(oversize.len(), MAX_WINDOW_BYTES.saturating_add(1));
    assert!(
        decode_budget_window(oversize.as_bytes())
            .as_ref()
            .is_empty()
    );
    let depth_at_limit = format!(
        "{}null{}",
        "{\"a\":".repeat(MAX_WINDOW_DEPTH),
        "}".repeat(MAX_WINDOW_DEPTH)
    );
    assert_eq!(
        decode_budget_window(depth_at_limit.as_bytes())
            .as_ref()
            .len(),
        1
    );
    let too_deep = MAX_WINDOW_DEPTH.saturating_add(1);
    let over_limit = format!("{}null{}", "{\"a\":".repeat(too_deep), "}".repeat(too_deep));
    assert!(
        decode_budget_window(over_limit.as_bytes())
            .as_ref()
            .is_empty()
    );
    assert!(decode_budget_window(b"not json").as_ref().is_empty());
    assert!(decode_budget_window(b"42").as_ref().is_empty());
    assert!(decode_budget_window(br#""text""#).as_ref().is_empty());
    assert!(
        decode_budget_window(br#"{"a":1,"a":2}"#)
            .as_ref()
            .is_empty()
    );
}

#[test]
fn nested_arrays_count_toward_the_depth_limit() {
    let nested = MAX_WINDOW_DEPTH.saturating_sub(1);
    let at_limit = format!("{{\"a\":{}null{}}}", "[".repeat(nested), "]".repeat(nested));
    assert_eq!(decode_budget_window(at_limit.as_bytes()).as_ref().len(), 1);
    let too_deep = format!(
        "{{\"a\":{}null{}}}",
        "[".repeat(MAX_WINDOW_DEPTH),
        "]".repeat(MAX_WINDOW_DEPTH)
    );
    assert!(
        decode_budget_window(too_deep.as_bytes())
            .as_ref()
            .is_empty()
    );
}

#[rstest::rstest]
fn replays_the_budget_window_corpus(#[files("fuzz/corpus/budget_window/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_budget_window(&bytes));
}
