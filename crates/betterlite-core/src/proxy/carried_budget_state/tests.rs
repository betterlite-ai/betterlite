use super::{KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::BTreeMap;

fn reset_at() -> DateTime<Utc> {
    "2026-01-15T10:00:00Z".parse().unwrap()
}

#[test]
fn replays_the_carried_budget_snapshot_contract() {
    let snapshot = KeyBudgetSnapshot {
        budget_reset_at: Some(reset_at()),
    };

    assert_eq!(
        snapshot.metadata_entries(),
        BTreeMap::from([(
            "user_api_key_budget_reset_at".to_owned(),
            json!("2026-01-15T10:00:00Z")
        )])
    );
    assert_eq!(
        serde_json::from_str::<KeyBudgetSnapshot>(
            r#"{"user_api_key_budget_reset_at":"2026-01-15T10:00:00Z"}"#,
        )
        .unwrap(),
        snapshot
    );
}

#[test]
fn snapshots_allow_missing_optional_fields_but_reject_unknown_direct_fields() {
    assert_eq!(
        serde_json::from_str::<TeamBudgetSnapshot>("{}").unwrap(),
        TeamBudgetSnapshot {
            budget_reset_at: None,
            max_budget: None,
        }
    );
    serde_json::from_str::<UserBudgetSnapshot>(r#"{"unexpected":true}"#).unwrap_err();
}

#[test]
fn from_metadata_filters_unknown_keys_and_rejects_invalid_values() {
    let metadata = BTreeMap::from([
        (
            "user_api_key_team_table_max_budget".to_owned(),
            json!("12345678901234567890.123456789"),
        ),
        ("unrelated".to_owned(), json!(true)),
    ]);
    let snapshot = TeamBudgetSnapshot::from_metadata(&metadata).unwrap();
    assert_eq!(
        snapshot.max_budget,
        Some(Decimal::from_str_exact("12345678901234567890.123456789").unwrap())
    );

    let invalid = BTreeMap::from([("user_api_key_org_spend".to_owned(), json!("not-a-decimal"))]);
    assert!(OrgBudgetSnapshot::from_metadata(&invalid).is_none());
}

#[test]
fn org_snapshot_deserializes_json_number_without_float_rounding() {
    let snapshot: OrgBudgetSnapshot = serde_json::from_str(
        r#"{
            "user_api_key_org_spend": 0.1234567890123456789012345678,
            "user_api_key_org_max_budget": 100.01
        }"#,
    )
    .unwrap();

    assert_eq!(
        snapshot.spend,
        Decimal::from_str_exact("0.1234567890123456789012345678").unwrap()
    );
    assert_eq!(
        snapshot.max_budget,
        Some(Decimal::from_str_exact("100.01").unwrap())
    );
}
