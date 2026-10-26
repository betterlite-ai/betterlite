#![cfg(test)]

use super::{
    BudgetSnapshot, KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use crate::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::fmt::Debug;

fn reset_at() -> DateTime<Utc> {
    "2026-01-15T10:00:00Z".parse().unwrap()
}

fn metadata(text: &str) -> BTreeMap<String, AgentJson> {
    serde_json::from_str(text).unwrap()
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

fn round_trip<Snapshot>(snapshot: &Snapshot, expected: &str)
where
    Snapshot: BudgetSnapshot + PartialEq + Debug,
{
    let entries = snapshot.metadata_entries().unwrap();
    assert_eq!(entries, metadata(expected));
    assert_eq!(Snapshot::from_metadata(&entries).as_ref(), Some(snapshot));
}

#[test]
fn key_snapshot_round_trips_under_its_alias() {
    round_trip(
        &KeyBudgetSnapshot {
            budget_reset_at: Some(reset_at()),
        },
        r#"{"user_api_key_budget_reset_at": "2026-01-15T10:00:00Z"}"#,
    );
}

#[test]
fn team_snapshot_round_trips_under_its_aliases() {
    round_trip(
        &TeamBudgetSnapshot {
            budget_reset_at: Some(reset_at()),
            max_budget: Some(decimal("300.0")),
        },
        r#"{
            "user_api_key_team_budget_reset_at": "2026-01-15T10:00:00Z",
            "user_api_key_team_table_max_budget": 300.0
        }"#,
    );
}

#[test]
fn user_snapshot_round_trips_under_its_aliases() {
    round_trip(
        &UserBudgetSnapshot {
            budget_reset_at: None,
            max_budget: Some(decimal("50.25")),
            user_alias: Some("Alice".to_owned()),
        },
        r#"{
            "user_api_key_user_budget_reset_at": null,
            "user_api_key_user_table_max_budget": 50.25,
            "user_api_key_user_alias": "Alice"
        }"#,
    );
}

#[test]
fn org_snapshot_round_trips_under_its_aliases() {
    round_trip(
        &OrgBudgetSnapshot {
            spend: decimal("3.0"),
            max_budget: None,
        },
        r#"{"user_api_key_org_spend": 3.0, "user_api_key_org_max_budget": null}"#,
    );
}

#[rstest]
#[case::key_without_reset("{}", KeyBudgetSnapshot::from_metadata)]
#[case::team_without_reset(
    r#"{"user_api_key_team_table_max_budget": 300.0}"#,
    TeamBudgetSnapshot::from_metadata
)]
#[case::user_with_alias_only(
    r#"{"user_api_key_user_alias": "Alice"}"#,
    UserBudgetSnapshot::from_metadata
)]
#[case::org_without_max_budget(
    r#"{"user_api_key_org_spend": 1.0}"#,
    OrgBudgetSnapshot::from_metadata
)]
#[case::org_with_null_spend(
    r#"{"user_api_key_org_spend": null, "user_api_key_org_max_budget": null}"#,
    OrgBudgetSnapshot::from_metadata
)]
#[case::org_with_text_spend(
    r#"{"user_api_key_org_spend": "not-a-decimal", "user_api_key_org_max_budget": null}"#,
    OrgBudgetSnapshot::from_metadata
)]
#[case::team_with_bad_reset(
    r#"{"user_api_key_team_budget_reset_at": "soon", "user_api_key_team_table_max_budget": null}"#,
    TeamBudgetSnapshot::from_metadata
)]
#[case::user_with_numeric_alias(
    r#"{
        "user_api_key_user_budget_reset_at": null,
        "user_api_key_user_table_max_budget": null,
        "user_api_key_user_alias": 5
    }"#,
    UserBudgetSnapshot::from_metadata
)]
fn from_metadata_gives_none_when_a_carried_field_is_missing_or_invalid<Snapshot>(
    #[case] carried: &str,
    #[case] from_metadata: fn(&BTreeMap<String, AgentJson>) -> Option<Snapshot>,
) where
    Snapshot: Debug,
{
    assert!(from_metadata(&metadata(carried)).is_none());
}

#[test]
fn from_metadata_accepts_null_and_offset_timestamps() {
    assert_eq!(
        KeyBudgetSnapshot::from_metadata(&metadata(r#"{"user_api_key_budget_reset_at": null}"#)),
        Some(KeyBudgetSnapshot {
            budget_reset_at: None
        })
    );
    assert_eq!(
        KeyBudgetSnapshot::from_metadata(&metadata(
            r#"{"user_api_key_budget_reset_at": "2026-01-15T12:00:00+02:00"}"#
        )),
        Some(KeyBudgetSnapshot {
            budget_reset_at: Some(reset_at())
        })
    );
}

#[test]
fn from_metadata_ignores_keys_it_does_not_carry() {
    let carried = metadata(
        r#"{
            "user_api_key_org_spend": 2.5,
            "user_api_key_org_max_budget": 10,
            "user_api_key_team_table_max_budget": 300.0,
            "requester_ip_address": "10.0.0.1"
        }"#,
    );
    assert_eq!(
        OrgBudgetSnapshot::from_metadata(&carried),
        Some(OrgBudgetSnapshot {
            spend: decimal("2.5"),
            max_budget: Some(decimal("10")),
        })
    );
}

#[test]
fn direct_deserialization_ignores_unknown_fields() {
    let snapshot = serde_json::from_str::<KeyBudgetSnapshot>(
        r#"{"user_api_key_budget_reset_at": null, "unexpected": true}"#,
    )
    .unwrap();
    assert_eq!(snapshot.budget_reset_at, None);
}

#[rstest]
#[case::long_float("0.1234567890123456789012345678", "0.1234567890123456789012345678")]
#[case::short_float("100.01", "100.01")]
#[case::integer("12", "12")]
#[case::numeric_text(
    r#""12345678901234567890.123456789""#,
    "12345678901234567890.123456789"
)]
fn money_reads_without_float_rounding(#[case] spend: &str, #[case] expected: &str) {
    let carried = metadata(&format!(
        r#"{{"user_api_key_org_spend": {spend}, "user_api_key_org_max_budget": {spend}}}"#
    ));
    let snapshot = OrgBudgetSnapshot::from_metadata(&carried).unwrap();
    assert_eq!(snapshot.spend, decimal(expected));
    assert_eq!(snapshot.max_budget, Some(decimal(expected)));
}
