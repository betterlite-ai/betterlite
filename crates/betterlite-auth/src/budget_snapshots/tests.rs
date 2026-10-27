#![cfg(test)]

use super::*;

fn raw(value: &str) -> Box<RawValue> {
    serde_json::from_str(value).unwrap()
}

#[test]
fn required_nullable_fields_and_aliases_are_serialized() {
    let snapshot = TeamBudgetSnapshot {
        budget_reset_at: None,
        max_budget: None,
    };
    let entries = snapshot.metadata_entries();
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
