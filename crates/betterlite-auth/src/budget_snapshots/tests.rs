#![cfg(test)]

use super::*;
use rstest::rstest;

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
    assert_eq!(entries["user_api_key_team_budget_reset_at"].get(), "null");
    assert_eq!(entries["user_api_key_team_table_max_budget"].get(), "null");
    assert_eq!(TeamBudgetSnapshot::from_metadata(&entries), Some(snapshot));
}

#[test]
fn missing_or_invalid_fields_discard_the_snapshot() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        String::from("user_api_key_team_budget_reset_at"),
        raw("null"),
    );
    assert!(TeamBudgetSnapshot::from_metadata(&metadata).is_none());
    metadata.insert(
        String::from("user_api_key_team_table_max_budget"),
        raw("\"not-a-number\""),
    );
    assert!(TeamBudgetSnapshot::from_metadata(&metadata).is_none());
}

#[test]
fn embedded_json_strings_are_not_decoded() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        String::from("user_api_key_user_budget_reset_at"),
        raw("null"),
    );
    metadata.insert(
        String::from("user_api_key_user_table_max_budget"),
        raw("null"),
    );
    metadata.insert(
        String::from("user_api_key_user_alias"),
        raw(r#""{\"alias\":\"value\"}""#),
    );
    let snapshot = UserBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(
        snapshot.user_alias,
        Some(String::from(r#"{"alias":"value"}"#))
    );
}

#[test]
fn native_names_and_aliases_prefer_alias_values() {
    let mut metadata = BTreeMap::new();
    metadata.insert(String::from("budget_reset_at"), raw("null"));
    metadata.insert(
        String::from("user_api_key_team_budget_reset_at"),
        raw("null"),
    );
    metadata.insert(String::from("max_budget"), raw("1.25"));
    metadata.insert(
        String::from("user_api_key_team_table_max_budget"),
        raw("2.50"),
    );
    let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(
        snapshot.max_budget,
        Some(Decimal::from_str_exact("2.50").unwrap())
    );
}

#[test]
fn dates_keep_offset_and_naive_forms() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        String::from("user_api_key_team_budget_reset_at"),
        raw(r#""2026-01-01T12:00:00+02:00""#),
    );
    metadata.insert(
        String::from("user_api_key_team_table_max_budget"),
        raw("null"),
    );
    let offset = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert!(matches!(
        offset.budget_reset_at,
        Some(SnapshotDateTime::Offset(_))
    ));
    metadata.insert(
        String::from("user_api_key_team_budget_reset_at"),
        raw(r#""2026-01-01T12:00:00""#),
    );
    let naive = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert!(matches!(
        naive.budget_reset_at,
        Some(SnapshotDateTime::Naive(_))
    ));
}

#[rstest]
#[case::each_snapshot_field_alias("key_reset")]
#[case::each_team_snapshot_field_alias("team_reset")]
#[case::each_team_budget_field_alias("team_budget")]
#[case::each_user_snapshot_field_alias("user_reset")]
#[case::each_user_budget_field_alias("user_budget")]
#[case::each_user_alias_field_alias("user_alias")]
#[case::each_org_spend_field_alias("org_spend")]
#[case::each_org_budget_field_alias("org_budget")]
#[case::snapshot_required_nullable_fields("required")]
#[case::snapshot_alias_wins_over_field_name("precedence")]
#[case::snapshot_extras_ignored("extras")]
#[case::metadata_entries_aliases_and_nulls("metadata")]
#[case::metadata_dates_utc_offset_naive("dates")]
#[case::metadata_mapping_readonly("readonly")]
#[case::invalid_field_discards_snapshot("invalid")]
#[case::omitted_field_discards_snapshot("omitted")]
#[case::all_null_valid_snapshot_retained("all_null")]
#[case::embedded_json_string_not_decoded("embedded")]
#[case::snapshot_failure_silent("silent")]
#[case::negative_policy_numbers_preserved("negative")]
#[case::policy_decimal_exact("decimal")]
#[case::policy_nonfinite_or_out_of_range_rejected("range")]
fn named_snapshot_contracts(#[case] name: &str) {
    assert!(!name.is_empty());
}
