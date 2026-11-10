#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::{BudgetSubject, subject_within_model_budget};
use crate::model_budget_limiter::tests::support::{
    DAY, FakeCache, amount, budgets, exceeded, limiter, limiter_at, limiter_with_status,
};

const KEY_SPEND: &str = "virtual_key_spend:key-1:gpt-4o:1d";
const KEY_LEGACY_SPEND: &str = "virtual_key_spend:key-1:openai/gpt-4o:1d";
const END_USER_SPEND: &str = "end_user_model_spend:end-user-1:gpt-4o:1d";
const END_USER_LEGACY_SPEND: &str = "end_user_model_spend:end-user-1:openai/gpt-4o:1d";
const USER_SPEND: &str = "user_model_spend:user-1:gpt-4o:1d";
const USER_LEGACY_SPEND: &str = "user_model_spend:user-1:openai/gpt-4o:1d";
const TEAM_SPEND: &str = "team_model_spend:team-1:gpt-4o:1d";
const TEAM_LEGACY_SPEND: &str = "team_model_spend:team-1:openai/gpt-4o:1d";

fn key<'input>(entity_id: &'input str, key_alias: Option<&'input str>) -> BudgetSubject<'input> {
    BudgetSubject::Key {
        entity_id,
        key_alias,
    }
}

fn user(user_id: &str) -> BudgetSubject<'_> {
    BudgetSubject::User { user_id }
}

fn end_user(end_user_id: &str) -> BudgetSubject<'_> {
    BudgetSubject::EndUser { end_user_id }
}

fn team<'input>(
    team_id: &'input str,
    key_model_max_budget: Option<&'input BTreeMap<String, JsonData>>,
) -> BudgetSubject<'input> {
    BudgetSubject::Team {
        team_id,
        key_model_max_budget,
    }
}

#[tokio::test]
async fn a_zero_cap_blocks_a_counter_that_was_never_written() {
    let subject = limiter(FakeCache::new());
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 0, "budget_duration": "1d"}}"#);
    let error = subject_within_model_budget(
        &subject,
        &key("key-1", Some("key-alias")),
        Some(&budget),
        "gpt-4o",
    )
    .await
    .unwrap_err();
    let details = exceeded(error);
    assert_eq!(details.current_cost, Decimal::ZERO);
    assert_eq!(details.max_budget, Decimal::ZERO);
    assert_eq!(details.entity_type.as_deref(), Some("key"));
    assert_eq!(details.entity_id.as_deref(), Some("key-1"));
    assert_eq!(details.status_code, 422);
}

#[tokio::test]
async fn the_configured_budget_exceeded_status_is_reported() {
    let subject = limiter_with_status(FakeCache::new(), 429);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 0, "budget_duration": "1d"}}"#);
    let error = subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).status_code, 429);
}

#[tokio::test]
async fn a_negative_cap_bypasses_the_counter() {
    let cache = FakeCache::new();
    cache.preload(KEY_SPEND, amount("100"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": -1, "budget_duration": "1d"}}"#);
    subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn an_unset_cap_bypasses_the_counter() {
    let cache = FakeCache::new();
    cache.preload(KEY_SPEND, amount("100"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": null, "budget_duration": "1d"}}"#);
    subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn an_empty_budget_map_passes_without_a_spend_read() {
    let subject = limiter(FakeCache::new());
    let budget = budgets("{}");
    subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn an_absent_budget_map_passes() {
    let subject = limiter(FakeCache::new());
    subject_within_model_budget(&subject, &key("key-1", None), None, "gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn a_model_the_budget_does_not_name_passes() {
    let subject = limiter(FakeCache::new());
    let budget = budgets(r#"{"gpt-4o-mini": {"max_budget": 0, "budget_duration": "1d"}}"#);
    subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn a_malformed_entry_is_skipped_like_an_absent_one() {
    let subject = limiter(FakeCache::new());
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 0}}"#);
    subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn spend_just_below_the_cap_passes_and_the_cap_itself_blocks() {
    let below = FakeCache::new();
    below.preload(KEY_SPEND, amount("4.99"));
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 5, "budget_duration": "1d"}}"#);
    subject_within_model_budget(
        &limiter(below),
        &key("key-1", None),
        Some(&budget),
        "gpt-4o",
    )
    .await
    .unwrap();
    let at = FakeCache::new();
    at.preload(KEY_SPEND, amount("5"));
    let error =
        subject_within_model_budget(&limiter(at), &key("key-1", None), Some(&budget), "gpt-4o")
            .await
            .unwrap_err();
    assert_eq!(exceeded(error).current_cost, amount("5"));
}

#[tokio::test]
async fn the_key_error_names_the_alias_and_the_model() {
    let subject = limiter(FakeCache::new());
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 0, "budget_duration": "1d"}}"#);
    let error = subject_within_model_budget(
        &subject,
        &key("key-1", Some("key-alias")),
        Some(&budget),
        "gpt-4o",
    )
    .await
    .unwrap_err();
    let message = exceeded(error).message.unwrap();
    assert!(message.contains("key-alias"));
    assert!(message.contains("gpt-4o"));
}

#[tokio::test]
async fn the_user_scope_reads_the_user_counter_and_names_the_user_entity() {
    let cache = FakeCache::new();
    cache.preload(USER_SPEND, amount("3"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 2, "budget_duration": "1d"}}"#);
    let error = subject_within_model_budget(&subject, &user("user-1"), Some(&budget), "gpt-4o")
        .await
        .unwrap_err();
    let details = exceeded(error);
    assert_eq!(details.current_cost, amount("3"));
    assert_eq!(details.entity_type.as_deref(), Some("user"));
    assert_eq!(details.entity_id.as_deref(), Some("user-1"));
}

#[tokio::test]
async fn the_end_user_scope_reads_the_end_user_counter_and_names_the_entity() {
    let cache = FakeCache::new();
    cache.preload(END_USER_SPEND, amount("3"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 2, "budget_duration": "1d"}}"#);
    let error =
        subject_within_model_budget(&subject, &end_user("end-user-1"), Some(&budget), "gpt-4o")
            .await
            .unwrap_err();
    let details = exceeded(error);
    assert_eq!(details.entity_type.as_deref(), Some("end_user"));
    assert_eq!(details.entity_id.as_deref(), Some("end-user-1"));
}

#[tokio::test]
async fn the_team_scope_reads_the_team_counter_and_names_the_entity() {
    let cache = FakeCache::new();
    cache.preload(TEAM_SPEND, amount("3"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 2, "budget_duration": "1d"}}"#);
    let error =
        subject_within_model_budget(&subject, &team("team-1", None), Some(&budget), "gpt-4o")
            .await
            .unwrap_err();
    let details = exceeded(error);
    assert_eq!(details.entity_type.as_deref(), Some("team"));
    assert_eq!(details.entity_id.as_deref(), Some("team-1"));
}

#[tokio::test]
async fn the_team_cap_is_skipped_when_the_key_gates_the_model() {
    let cache = FakeCache::new();
    cache.preload(TEAM_SPEND, amount("5"));
    let subject = limiter(cache);
    let team_budget = budgets(r#"{"gpt-4o": {"max_budget": 5, "budget_duration": "1d"}}"#);
    let key_budget = budgets(r#"{"gpt-4o": {"max_budget": 10, "budget_duration": "1d"}}"#);
    subject_within_model_budget(
        &subject,
        &team("team-1", Some(&key_budget)),
        Some(&team_budget),
        "gpt-4o",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_team_cap_applies_when_the_key_budget_does_not_gate_the_model() {
    let cache = FakeCache::new();
    cache.preload(TEAM_SPEND, amount("5"));
    let subject = limiter(cache);
    let team_budget = budgets(r#"{"gpt-4o": {"max_budget": 5, "budget_duration": "1d"}}"#);
    let key_budget = budgets(r#"{"gpt-4o": {"max_budget": -1, "budget_duration": "1d"}}"#);
    let refusal = subject_within_model_budget(
        &subject,
        &team("team-1", Some(&key_budget)),
        Some(&team_budget),
        "gpt-4o",
    )
    .await;
    assert!(refusal.is_err());
}

#[tokio::test]
async fn the_team_cap_applies_when_the_key_budget_names_another_model() {
    let cache = FakeCache::new();
    cache.preload(TEAM_SPEND, amount("5"));
    let subject = limiter(cache);
    let team_budget = budgets(r#"{"gpt-4o": {"max_budget": 5, "budget_duration": "1d"}}"#);
    let key_budget = budgets(r#"{"gpt-4o-mini": {"max_budget": 0, "budget_duration": "1d"}}"#);
    let refusal = subject_within_model_budget(
        &subject,
        &team("team-1", Some(&key_budget)),
        Some(&team_budget),
        "gpt-4o",
    )
    .await;
    assert!(refusal.is_err());
}

#[tokio::test]
async fn the_key_carry_adds_the_counter_billed_to_the_request_model() {
    let cache = FakeCache::new();
    cache.preload(KEY_SPEND, amount("2"));
    cache.preload(KEY_LEGACY_SPEND, amount("3"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 4.5, "budget_duration": "1d"}}"#);
    let error = subject_within_model_budget(
        &subject,
        &key("key-1", None),
        Some(&budget),
        "openai/gpt-4o",
    )
    .await
    .unwrap_err();
    assert_eq!(exceeded(error).current_cost, amount("5"));
}

#[tokio::test]
async fn the_end_user_carry_adds_the_counter_billed_to_the_request_model() {
    let cache = FakeCache::new();
    cache.preload(END_USER_SPEND, amount("2"));
    cache.preload(END_USER_LEGACY_SPEND, amount("3"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 4.5, "budget_duration": "1d"}}"#);
    let error = subject_within_model_budget(
        &subject,
        &end_user("end-user-1"),
        Some(&budget),
        "openai/gpt-4o",
    )
    .await
    .unwrap_err();
    assert_eq!(exceeded(error).current_cost, amount("5"));
}

#[tokio::test]
async fn the_user_scope_reads_no_legacy_counter() {
    let cache = FakeCache::new();
    cache.preload(USER_SPEND, amount("1"));
    cache.preload(USER_LEGACY_SPEND, amount("9"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 2, "budget_duration": "1d"}}"#);
    subject_within_model_budget(&subject, &user("user-1"), Some(&budget), "openai/gpt-4o")
        .await
        .unwrap();
}

#[tokio::test]
async fn the_team_scope_reads_no_legacy_counter() {
    let cache = FakeCache::new();
    cache.preload(TEAM_SPEND, amount("1"));
    cache.preload(TEAM_LEGACY_SPEND, amount("9"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 2, "budget_duration": "1d"}}"#);
    subject_within_model_budget(
        &subject,
        &team("team-1", None),
        Some(&budget),
        "openai/gpt-4o",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_carry_applies_until_the_window_has_fully_passed() {
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 4.5, "budget_duration": "1d"}}"#);
    let cache = FakeCache::new();
    cache.preload(KEY_SPEND, amount("2"));
    cache.preload(KEY_LEGACY_SPEND, amount("3"));
    let just_inside = limiter_at(cache, Duration::from_secs(86_399));
    let refusal = subject_within_model_budget(
        &just_inside,
        &key("key-1", None),
        Some(&budget),
        "openai/gpt-4o",
    )
    .await;
    assert!(refusal.is_err());
    let expired = FakeCache::new();
    expired.preload(KEY_SPEND, amount("2"));
    expired.preload(KEY_LEGACY_SPEND, amount("3"));
    let at_window = limiter_at(expired, Duration::from_secs(DAY));
    subject_within_model_budget(
        &at_window,
        &key("key-1", None),
        Some(&budget),
        "openai/gpt-4o",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_same_request_and_budget_model_does_not_count_the_counter_twice() {
    let cache = FakeCache::new();
    cache.preload(KEY_SPEND, amount("3"));
    let subject = limiter(cache);
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 4.5, "budget_duration": "1d"}}"#);
    subject_within_model_budget(&subject, &key("key-1", None), Some(&budget), "gpt-4o")
        .await
        .unwrap();
}
