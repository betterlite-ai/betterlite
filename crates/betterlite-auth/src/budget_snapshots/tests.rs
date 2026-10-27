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
#[case::each_snapshot_field_alias("key_reset", r#"{"user_api_key_budget_reset_at":null}"#)]
#[case::each_team_snapshot_field_alias(
    "team_reset",
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#
)]
#[case::each_team_budget_field_alias(
    "team_budget",
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":1.25}"#
)]
#[case::each_user_snapshot_field_alias("user_reset", r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":null}"#)]
#[case::each_user_budget_field_alias("user_budget", r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":1.25,"user_api_key_user_alias":null}"#)]
#[case::each_user_alias_field_alias("user_alias", r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":"u"}"#)]
#[case::each_org_spend_field_alias(
    "org_spend",
    r#"{"user_api_key_org_spend":1.25,"user_api_key_org_max_budget":null}"#
)]
#[case::each_org_budget_field_alias(
    "org_budget",
    r#"{"user_api_key_org_spend":0,"user_api_key_org_max_budget":1.25}"#
)]
#[case::snapshot_required_nullable_fields(
    "required",
    r#"{"user_api_key_team_budget_reset_at":null}"#
)]
#[case::snapshot_alias_wins_over_field_name("precedence", r#"{"budget_reset_at":null,"user_api_key_team_budget_reset_at":null,"max_budget":1.25,"user_api_key_team_table_max_budget":2.5}"#)]
#[case::snapshot_extras_ignored("extras", r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null,"unrelated":1}"#)]
#[case::metadata_entries_aliases_and_nulls(
    "metadata",
    r#"{"budget_reset_at":null,"max_budget":null}"#
)]
#[case::metadata_dates_utc_offset_naive(
    "dates",
    r#"{"budget_reset_at":"2026-01-01T12:00:00+02:00","max_budget":null}"#
)]
#[case::metadata_mapping_readonly("readonly", r#"{"budget_reset_at":null,"max_budget":null}"#)]
#[case::invalid_field_discards_snapshot(
    "invalid",
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":"bad"}"#
)]
#[case::omitted_field_discards_snapshot("omitted", r#"{"user_api_key_team_budget_reset_at":null}"#)]
#[case::all_null_valid_snapshot_retained(
    "all_null",
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":null}"#
)]
#[case::embedded_json_string_not_decoded("embedded", r#"{"user_api_key_user_budget_reset_at":null,"user_api_key_user_table_max_budget":null,"user_api_key_user_alias":"{\"x\":1}"}"#)]
#[case::snapshot_failure_silent(
    "silent",
    r#"{"user_api_key_team_budget_reset_at":null,"user_api_key_team_table_max_budget":"bad"}"#
)]
#[case::negative_policy_numbers_preserved(
    "negative",
    r#"{"user_api_key_org_spend":-1,"user_api_key_org_max_budget":-2}"#
)]
#[case::policy_decimal_exact(
    "decimal",
    r#"{"user_api_key_org_spend":1.2300,"user_api_key_org_max_budget":null}"#
)]
#[case::policy_nonfinite_or_out_of_range_rejected(
    "range",
    r#"{"user_api_key_org_spend":1e+1000,"user_api_key_org_max_budget":null}"#
)]
fn named_snapshot_contracts(#[case] name: &str, #[case] input: &str) {
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_str(input).unwrap();
    match name {
        "key_reset" => assert!(KeyBudgetSnapshot::from_metadata(&metadata).is_some()),
        "team_reset" | "team_budget" | "extras" | "all_null" => {
            assert!(TeamBudgetSnapshot::from_metadata(&metadata).is_some());
        }
        "user_reset" | "user_budget" | "user_alias" | "embedded" => {
            assert!(UserBudgetSnapshot::from_metadata(&metadata).is_some());
        }
        "org_spend" | "org_budget" | "negative" | "decimal" => {
            assert!(OrgBudgetSnapshot::from_metadata(&metadata).is_some());
        }
        "required" | "invalid" | "omitted" | "silent" | "range" => {
            assert!(TeamBudgetSnapshot::from_metadata(&metadata).is_none());
        }
        "precedence" => {
            let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
            assert_eq!(
                snapshot.max_budget,
                Some(Decimal::from_str_exact("2.5").unwrap())
            );
        }
        "metadata" => {
            let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
            assert_eq!(
                snapshot.metadata_entries().unwrap()["user_api_key_team_budget_reset_at"].get(),
                "null"
            );
        }
        "dates" => {
            let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
            assert!(matches!(
                snapshot.budget_reset_at,
                Some(SnapshotDateTime::Offset(_))
            ));
        }
        "readonly" => {
            TeamBudgetSnapshot::from_metadata(&metadata)
                .unwrap()
                .metadata_entries()
                .unwrap();
        }
        _ => panic!("unknown snapshot case"),
    }
}
