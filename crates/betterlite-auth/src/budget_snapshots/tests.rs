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

fn parse_metadata(json: &str) -> BTreeMap<String, Box<RawValue>> {
    serde_json::from_str(json).unwrap()
}

fn alias_value<T: super::BudgetSnapshot>(
    metadata: &BTreeMap<String, Box<RawValue>>,
    key: &str,
) -> Option<String> {
    let snapshot = T::from_metadata(metadata)?;
    let entries = snapshot.metadata_entries().unwrap();
    entries
        .as_ref()
        .get(key)
        .map(|value| value.get().to_owned())
}

fn parses<T: super::BudgetSnapshot>(metadata: &BTreeMap<String, Box<RawValue>>) -> bool {
    T::from_metadata(metadata).is_some()
}

#[rstest]
#[case::key_budget_reset_at(
    alias_value::<KeyBudgetSnapshot>,
    r#"{"budget_reset_at":null}"#,
    r#"{"user_api_key_budget_reset_at":null}"#,
    "user_api_key_budget_reset_at"
)]
#[case::team_budget_reset_at(
    alias_value::<TeamBudgetSnapshot>,
    r#"{"budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#,
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#,
    "user_api_key_team_budget_reset_at"
)]
#[case::team_max_budget(
    alias_value::<TeamBudgetSnapshot>,
    r#"{"user_api_key_team_budget_reset_at":null,"max_budget":1.5}"#,
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":1.5}"#,
    "user_api_key_team_table_max_budget"
)]
#[case::user_budget_reset_at(
    alias_value::<UserBudgetSnapshot>,
    r#"{"budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#,
    "user_api_key_user_budget_reset_at"
)]
#[case::user_max_budget(
    alias_value::<UserBudgetSnapshot>,
    r#"{"user_api_key_user_budget_reset_at":null,"max_budget":2.5,"user_api_key_user_alias":null}"#,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":2.5,"user_api_key_user_alias":null}"#,
    "user_api_key_user_table_max_budget"
)]
#[case::user_user_alias(
    alias_value::<UserBudgetSnapshot>,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_alias":"name"}"#,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":"name"}"#,
    "user_api_key_user_alias"
)]
#[case::org_spend(
    alias_value::<OrgBudgetSnapshot>,
    r#"{"spend":1,"user_api_key_org_max_budget":null}"#,
    r#"{"user_api_key_org_spend":1,"user_api_key_org_max_budget":null}"#,
    "user_api_key_org_spend"
)]
#[case::org_max_budget(
    alias_value::<OrgBudgetSnapshot>,
    r#"{"user_api_key_org_spend":1,"max_budget":3}"#,
    r#"{"user_api_key_org_spend":1,"user_api_key_org_max_budget":3}"#,
    "user_api_key_org_max_budget"
)]
fn each_snapshot_field_alias(
    #[case] resolver: fn(&BTreeMap<String, Box<RawValue>>, &str) -> Option<String>,
    #[case] native_json: &str,
    #[case] alias_json: &str,
    #[case] alias_key: &str,
) {
    let native_value =
        resolver(&parse_metadata(native_json), alias_key).expect("native spelling must parse");
    let alias_value =
        resolver(&parse_metadata(alias_json), alias_key).expect("alias spelling must parse");
    assert_eq!(native_value, alias_value);
}

#[rstest]
#[case::team(
    alias_value::<TeamBudgetSnapshot>,
    r#"{"budget_reset_at":null,"user_api_key_team_budget_reset_at":null,"max_budget":1.25,"user_api_key_team_table_max_budget":2.5}"#,
    "user_api_key_team_table_max_budget",
    "2.5"
)]
#[case::user(
    alias_value::<UserBudgetSnapshot>,
    r#"{"budget_reset_at":null,"user_api_key_user_budget_reset_at":null,"max_budget":1.25,"user_api_key_user_table_max_budget":2.5,"user_alias":"native","user_api_key_user_alias":"alias"}"#,
    "user_api_key_user_table_max_budget",
    "2.5"
)]
#[case::org(
    alias_value::<OrgBudgetSnapshot>,
    r#"{"spend":1,"user_api_key_org_spend":2,"max_budget":1.25,"user_api_key_org_max_budget":2.5}"#,
    "user_api_key_org_spend",
    "2"
)]
fn snapshot_alias_wins_over_field_name(
    #[case] resolver: fn(&BTreeMap<String, Box<RawValue>>, &str) -> Option<String>,
    #[case] input: &str,
    #[case] alias_key: &str,
    #[case] expected: &str,
) {
    let value = resolver(&parse_metadata(input), alias_key).expect("alias spelling must win");
    assert_eq!(value, expected);
}

#[rstest]
#[case::key_budget_reset_at(
    parses::<KeyBudgetSnapshot>,
    "{}",
    r#"{"user_api_key_budget_reset_at":null}"#,
    true
)]
#[case::team_budget_reset_at(
    parses::<TeamBudgetSnapshot>,
    r#"{"user_api_key_team_table_max_budget":null}"#,
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#,
    true
)]
#[case::team_max_budget(
    parses::<TeamBudgetSnapshot>,
    r#"{"user_api_key_team_budget_reset_at":null}"#,
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#,
    true
)]
#[case::user_budget_reset_at(
    parses::<UserBudgetSnapshot>,
    r#"{"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#,
    true
)]
#[case::user_max_budget(
    parses::<UserBudgetSnapshot>,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_alias":null}"#,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#,
    true
)]
#[case::user_user_alias(
    parses::<UserBudgetSnapshot>,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null}"#,
    r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#,
    true
)]
#[case::org_spend(
    parses::<OrgBudgetSnapshot>,
    r#"{"user_api_key_org_max_budget":null}"#,
    r#"{"user_api_key_org_spend":null,"user_api_key_org_max_budget":null}"#,
    false
)]
#[case::org_max_budget(
    parses::<OrgBudgetSnapshot>,
    r#"{"user_api_key_org_spend":1}"#,
    r#"{"user_api_key_org_spend":1,"user_api_key_org_max_budget":null}"#,
    true
)]
fn snapshot_field_omission_and_null_discard_rules(
    #[case] parses: fn(&BTreeMap<String, Box<RawValue>>) -> bool,
    #[case] omitted_json: &str,
    #[case] null_json: &str,
    #[case] null_is_accepted: bool,
) {
    assert!(
        !parses(&parse_metadata(omitted_json)),
        "omitting a required field must discard the snapshot"
    );
    assert_eq!(parses(&parse_metadata(null_json)), null_is_accepted);
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
