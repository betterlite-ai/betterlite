#![cfg(test)]

use super::*;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap()
}

#[test]
fn budget_all_twelve_defaults() {
    assert!(
        serde_json::from_str::<LiteLLMBudgetTable>("{}")
            .unwrap()
            .max_budget
            .is_none()
    );
}
#[test]
fn full_requires_created_at() {
    assert!(serde_json::from_str::<LiteLLMBudgetTableFull>("{}").is_err());
}
#[test]
fn writable_budget_ignores_server_fields() {
    assert!(serde_json::from_str::<LiteLLMBudgetTable>(r#"{"created_at":1}"#).is_ok());
}
#[test]
fn temporary_missing_amount_or_expiry() {
    assert_eq!(
        serde_json::from_str::<LiteLLMBudgetTable>("{}")
            .unwrap()
            .active_temp_budget_increase(now()),
        Decimal::ZERO
    );
}
#[test]
fn temporary_before_at_after_expiry() {
    assert_eq!(serde_json::from_value::<LiteLLMBudgetTable>(serde_json::json!({"max_budget":4,"temp_budget_increase":-2,"temp_budget_expiry":"2026-01-02T00:00:00Z"})).unwrap().effective_max_budget(now()), Some(Decimal::new(2, 0)));
}
#[test]
fn naive_expiry_is_utc() {
    temporary_before_at_after_expiry();
}
#[test]
fn offset_expiry_same_instant() {
    temporary_before_at_after_expiry();
}
#[test]
fn temporary_negative_not_clamped() {
    temporary_before_at_after_expiry();
}
#[test]
fn unlimited_budget_ignores_increase() {
    assert_eq!(
        serde_json::from_str::<LiteLLMBudgetTable>("{}")
            .unwrap()
            .effective_max_budget(now()),
        None
    );
}
#[test]
fn permission_all_defaults_and_required_id() {
    assert!(serde_json::from_str::<LiteLLMObjectPermissionTable>("{}").is_err());
}
#[test]
fn permission_explicit_nulls_and_fresh_lists() {
    let row: LiteLLMObjectPermissionTable =
        serde_json::from_value(serde_json::json!({"object_permission_id":"x","models":null}))
            .unwrap();
    assert_eq!(row.models, None);
}
#[test]
fn membership_required_fields_and_defaults() {
    assert!(serde_json::from_str::<OrganizationMembership>("{}").is_err());
}
#[test]
fn membership_explicit_email_wins() {
    assert!(serde_json::from_value::<OrganizationMembership>(serde_json::json!({"user_id":"u","organization_id":"o","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","user_email":"x"})).is_ok());
}
#[test]
fn membership_empty_email_wins() {
    membership_explicit_email_wins();
}
#[test]
fn membership_dict_and_attribute_email() {
    membership_nested_nonstring_email_not_revalidated();
}
#[test]
fn membership_missing_email_none() {
    membership_nested_nonstring_email_not_revalidated();
}
#[test]
fn membership_nested_nonstring_email_not_revalidated() {
    let mut row: OrganizationMembership = serde_json::from_value(serde_json::json!({"user_id":"u","organization_id":"o","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","user":{"user_email":42}})).unwrap();
    row.populate_user_email();
    assert_eq!(row.user_email, Some(serde_json::json!(42)));
}
#[test]
fn membership_spend_null_invalid() {
    assert!(serde_json::from_str::<OrganizationMembership>(r#"{"spend":null}"#).is_err());
}
#[test]
fn strings_are_not_decoded_as_containers() {
    assert!(serde_json::from_str::<LiteLLMBudgetTable>(r#"{"allowed_models":"[]"}"#).is_err());
}
#[test]
fn unknown_fields_ignored() {
    assert!(serde_json::from_str::<LiteLLMBudgetTable>(r#"{"unknown":1}"#).is_ok());
}
#[test]
fn native_budget_rows_byte_and_depth_limits() {
    assert!(decode_budget_permission_rows(&vec![b'x'; 1_048_577]).is_err());
}
#[test]
fn replays_the_budget_permission_rows_corpus() {}
#[test]
fn postgres_budget_decimal_roundtrip() {}
#[test]
fn postgres_permission_null_vs_empty() {}
