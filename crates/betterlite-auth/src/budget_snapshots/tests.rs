#![cfg(test)]

use super::lax_datetime::LaxDateTime;
use super::{
    BudgetSnapshot as _, KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot,
    UserBudgetSnapshot,
};
use crate::agent_policy::AgentResponse;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

fn raw(value: &str) -> Box<RawValue> {
    serde_json::from_str(value).unwrap()
}

#[test]
fn required_nullable_fields_and_aliases_are_serialized() {
    let snapshot = TeamBudgetSnapshot {
        budget_reset_at: None,
        max_budget: None,
    };
    let entries = snapshot.metadata_entries().unwrap();
    assert_eq!(
        entries.as_ref()["user_api_key_team_budget_reset_at"].get(),
        "null"
    );
    assert_eq!(
        entries.as_ref()["user_api_key_team_table_max_budget"].get(),
        "null"
    );
    assert_eq!(
        TeamBudgetSnapshot::from_metadata(entries.as_ref()),
        Some(snapshot)
    );
}

#[test]
fn dates_keep_offset_and_naive_forms() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        String::from("user_api_key_team_budget_reset_at"),
        raw(r#""2026-01-01T12:00:00.4+02:00""#),
    );
    metadata.insert(
        String::from("user_api_key_team_table_max_budget"),
        raw("null"),
    );
    let offset = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert!(matches!(
        offset.budget_reset_at,
        Some(LaxDateTime::Offset(_))
    ));
    assert_eq!(
        offset.metadata_entries().unwrap().as_ref()["user_api_key_team_budget_reset_at"].get(),
        r#""2026-01-01T12:00:00.400000+02:00""#
    );
    metadata.insert(
        String::from("user_api_key_team_budget_reset_at"),
        raw(r#""2026-01-01T12:00:00""#),
    );
    let naive = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert!(matches!(naive.budget_reset_at, Some(LaxDateTime::Naive(_))));
}

#[test]
fn each_snapshot_field_alias() {
    let metadata: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(r#"{"user_api_key_budget_reset_at":null}"#).unwrap();
    assert!(KeyBudgetSnapshot::from_metadata(&metadata).is_some());
}

#[test]
fn snapshot_alias_wins_over_field_name() {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"budget_reset_at":null,"user_api_key_team_budget_reset_at":null,"max_budget":1.25,"user_api_key_team_table_max_budget":2.5}"#,
    )
    .unwrap();
    let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(
        snapshot.max_budget,
        Some(Decimal::from_str_exact("2.5").unwrap())
    );
}

#[rstest]
#[case::snapshot_extras_ignored(
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null,"unrelated":1}"#
)]
#[case::all_null_valid_snapshot_retained(
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#
)]
fn snapshot_accepted_with_extras_or_all_null(#[case] input: &str) {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(input).unwrap();
    assert!(TeamBudgetSnapshot::from_metadata(&metadata).is_some());
}

#[rstest]
#[case::snapshot_required_nullable_fields(r#"{"user_api_key_team_budget_reset_at":null}"#)]
#[case::invalid_field_discards_snapshot(
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":"bad"}"#
)]
#[case::omitted_field_discards_snapshot(r#"{"user_api_key_team_budget_reset_at":null}"#)]
#[case::snapshot_failure_silent(
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":"bad"}"#
)]
fn snapshot_discarded_on_missing_or_invalid_field(#[case] input: &str) {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(input).unwrap();
    assert!(TeamBudgetSnapshot::from_metadata(&metadata).is_none());
}

#[test]
fn metadata_entries_aliases_and_nulls() {
    let metadata: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(r#"{"budget_reset_at":null,"max_budget":null}"#).unwrap();
    let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(
        snapshot.metadata_entries().unwrap().as_ref()["user_api_key_team_budget_reset_at"].get(),
        "null"
    );
}

#[test]
fn metadata_dates_utc_offset_naive() {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"budget_reset_at":"2026-01-01T12:00:00+02:00","max_budget":null}"#,
    )
    .unwrap();
    let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(
        snapshot.metadata_entries().unwrap().as_ref()["user_api_key_team_budget_reset_at"].get(),
        r#""2026-01-01T12:00:00+02:00""#
    );
}

#[test]
fn metadata_mapping_readonly() {
    let metadata: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(r#"{"budget_reset_at":null,"max_budget":null}"#).unwrap();
    let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    let entries = snapshot.metadata_entries().unwrap();
    let shared: &BTreeMap<String, Box<RawValue>> = entries.as_ref();
    assert_eq!(shared["user_api_key_team_table_max_budget"].get(), "null");
    assert_eq!(shared.len(), 2);
}

#[test]
fn embedded_json_string_not_decoded() {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":"{\"x\":1}"}"#,
    )
    .unwrap();
    let snapshot = UserBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(snapshot.user_alias.as_deref(), Some(r#"{"x":1}"#));
    assert_eq!(
        snapshot.metadata_entries().unwrap().as_ref()["user_api_key_user_alias"].get(),
        r#""{\"x\":1}""#
    );
}

#[test]
fn negative_policy_numbers_preserved() {
    let metadata: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(r#"{"user_api_key_org_spend":-1,"user_api_key_org_max_budget":-2}"#)
            .unwrap();
    let snapshot = OrgBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(snapshot.spend.to_string(), "-1");
    assert_eq!(snapshot.max_budget, Some(Decimal::from(-2_i32)));
    let entries = snapshot.metadata_entries().unwrap();
    assert_eq!(entries.as_ref()["user_api_key_org_spend"].get(), "-1");
    assert_eq!(entries.as_ref()["user_api_key_org_max_budget"].get(), "-2");
}

#[test]
fn policy_decimal_exact() {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"user_api_key_org_spend":1.2300,"user_api_key_org_max_budget":null}"#,
    )
    .unwrap();
    let snapshot = OrgBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(snapshot.spend.to_string(), "1.2300");
    assert_eq!(snapshot.max_budget, None);
    let entries = snapshot.metadata_entries().unwrap();
    assert_eq!(entries.as_ref()["user_api_key_org_spend"].get(), "1.2300");
    assert_eq!(
        entries.as_ref()["user_api_key_org_max_budget"].get(),
        "null"
    );
}

#[test]
fn policy_nonfinite_or_out_of_range_rejected() {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"user_api_key_org_spend":1e+1000,"user_api_key_org_max_budget":null}"#,
    )
    .unwrap();
    assert!(OrgBudgetSnapshot::from_metadata(&metadata).is_none());
}

#[rstest]
#[case::policy_nonfinite_or_out_of_range_rejected(
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{},"spend":1e+1000}"#
)]
#[case::policy_nonfinite_or_out_of_range_rejected(
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{},"search_score":1e+1000}"#
)]
fn agent_response_rejects_nonfinite_numbers(#[case] input: &str) {
    serde_json::from_str::<AgentResponse>(input).unwrap_err();
}
