use rstest::rstest;

use super::support::{Fixture, auth, dec, decoded_row, only};

#[rstest]
#[case::no_token(r#"{"key_name":"k"}"#)]
#[case::zero_budget(r#"{"token":"tok","max_budget":0}"#)]
#[case::negative_budget(r#"{"token":"tok","max_budget":-3}"#)]
#[tokio::test]
async fn key_counter_skips_missing_token_and_nonpositive_budgets(#[case] token: &str) {
    let mut fixture = Fixture::new();
    fixture.token = auth(token);
    assert!(fixture.counters().await.is_empty());
}

#[tokio::test]
async fn key_counter_keeps_the_positive_cap_and_its_spend() {
    let mut fixture = Fixture::new();
    fixture.token = auth(r#"{"token":"tok","max_budget":25.5,"spend":1.5}"#);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:key:tok");
    assert_eq!(counter.max_budget, dec("25.5"));
    assert_eq!(counter.fallback_spend, dec("1.5"));
    assert_eq!(counter.entity_type.wire_value(), "key");
    assert_eq!(counter.entity_id, "tok");
    assert_eq!(counter.source_cache_key.as_deref(), Some("tok"));
    assert!(counter.spend_log_entity_id.is_none());
    assert!(counter.window_duration.is_none());
    assert!(counter.window_start.is_none());
}

#[tokio::test]
async fn key_window_counters_need_no_entity_cap() {
    let mut fixture = Fixture::new();
    fixture.token =
        auth(r#"{"token":"tok","budget_limits":[{"budget_duration":"1d","max_budget":5}]}"#);
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:key:tok:window:1d");
}

#[rstest]
#[case::no_team(None)]
#[case::zero_budget(Some(r#"{"team_id":"team-a","max_budget":0}"#))]
#[case::negative_budget(Some(r#"{"team_id":"team-a","max_budget":-3}"#))]
#[tokio::test]
async fn team_counter_skips_missing_and_nonpositive_caps(#[case] row: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.team = row.map(decoded_row);
    assert!(fixture.counters().await.is_empty());
}

#[tokio::test]
async fn team_counter_keeps_the_positive_cap_and_its_spend() {
    let mut fixture = Fixture::new();
    fixture.team = Some(decoded_row(
        r#"{"team_id":"team-a","max_budget":30,"spend":2.5}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:team:team-a");
    assert_eq!(counter.max_budget, dec("30"));
    assert_eq!(counter.fallback_spend, dec("2.5"));
    assert_eq!(counter.entity_type.wire_value(), "team");
    assert_eq!(counter.entity_id, "team-a");
    assert_eq!(counter.source_cache_key.as_deref(), Some("team_id:team-a"));
}

#[tokio::test]
async fn user_conditional_applies_without_a_team_key() {
    let mut fixture = Fixture::new();
    fixture.user = Some(decoded_row(
        r#"{"user_id":"user-a","max_budget":7,"spend":1}"#,
    ));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:user:user-a");
    assert_eq!(counter.max_budget, dec("7"));
    assert_eq!(counter.fallback_spend, dec("1"));
    assert_eq!(counter.entity_type.wire_value(), "user");
    assert_eq!(counter.source_cache_key.as_deref(), Some("user-a"));
}

#[rstest]
#[case::no_user(None)]
#[case::zero_budget(Some(r#"{"user_id":"user-a","max_budget":0}"#))]
#[case::negative_budget(Some(r#"{"user_id":"user-a","max_budget":-2}"#))]
#[tokio::test]
async fn user_conditional_skips_missing_or_nonpositive(#[case] row: Option<&str>) {
    let mut fixture = Fixture::new();
    fixture.user = row.map(decoded_row);
    assert!(fixture.counters().await.is_empty());
}

#[tokio::test]
async fn user_team_key_skips_the_user_budget() {
    let mut fixture = Fixture::new();
    fixture.team = Some(decoded_row(r#"{"team_id":"team-a"}"#));
    fixture.user = Some(decoded_row(r#"{"user_id":"user-a","max_budget":7}"#));
    assert!(fixture.counters().await.is_empty());
}

#[tokio::test]
async fn user_override_restores_the_user_budget_for_team_keys() {
    let mut fixture = Fixture::new();
    fixture.apply_override = true;
    fixture.team = Some(decoded_row(r#"{"team_id":"team-a"}"#));
    fixture.user = Some(decoded_row(r#"{"user_id":"user-a","max_budget":7}"#));
    let list = fixture.counters().await;
    let counter = only(&list);
    assert_eq!(counter.counter_key, "spend:user:user-a");
}
