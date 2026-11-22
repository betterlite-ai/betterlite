#![cfg(test)]

use betterlite_db::identity_rows::Payload;
use tracing::Level;

use super::super::defaults::{
    DefaultBudgetLookup, KeyEndUserDefaults, MemberBudgetError, key_end_user_budget_id,
};
use super::support::{
    BudgetAnswer, ScriptedBudgetRows, ScriptedCache, Surface, budget_fixture, capture_events,
    events_at, money, user_key,
};
use crate::auth_cache::payload::PayloadError;

fn payload(text: &str) -> Payload {
    serde_json::from_str(text).expect("the metadata payload decodes")
}

fn lookup<'deps>(
    rows: Option<&'deps ScriptedBudgetRows>,
    cache: &'deps ScriptedCache,
    proxy: Option<&'deps str>,
) -> DefaultBudgetLookup<'deps, ScriptedBudgetRows, ScriptedCache> {
    DefaultBudgetLookup {
        rows,
        cache,
        max_end_user_budget_id: proxy,
    }
}

#[test]
fn metadata_string_only() {
    assert_eq!(key_end_user_budget_id(None), None);
    assert_eq!(key_end_user_budget_id(Some(&payload("{}"))), None);
    assert_eq!(
        key_end_user_budget_id(Some(&payload(r#"{"end_user_budget_id": 5}"#))),
        None
    );
    assert_eq!(
        key_end_user_budget_id(Some(&payload(r#"{"end_user_budget_id": true}"#))),
        None
    );
    assert_eq!(
        key_end_user_budget_id(Some(&payload(r#"{"end_user_budget_id": [1]}"#))),
        None
    );
    assert_eq!(
        key_end_user_budget_id(Some(&payload(r#"{"end_user_budget_id": ""}"#))),
        None
    );
    assert_eq!(
        key_end_user_budget_id(Some(&payload(r#"{"end_user_budget_id": "b1"}"#))),
        Some("b1")
    );

    let key = user_key(r#"{"metadata": {"end_user_budget_id": "b1"}, "end_user_max_budget": 5}"#);
    assert_eq!(
        KeyEndUserDefaults::of_key(&key),
        KeyEndUserDefaults {
            key_end_user_budget_id: Some("b1".to_owned()),
            token_end_user_max_budget: Some(money("5")),
        }
    );
}

#[test]
fn whitespace_id_kept() {
    assert_eq!(
        key_end_user_budget_id(Some(&payload(r#"{"end_user_budget_id": " b1 "}"#))),
        Some(" b1 ")
    );
    let key = user_key(r#"{"metadata": {"end_user_budget_id": " b1 "}}"#);
    assert_eq!(
        KeyEndUserDefaults::of_key(&key).key_end_user_budget_id,
        Some(" b1 ".to_owned())
    );
}

#[tokio::test]
async fn explicit_empty_id_overrides() {
    let rows =
        ScriptedBudgetRows::answering(BudgetAnswer::Row(Some(budget_fixture("from-empty", None))));
    let cache = ScriptedCache::new();
    let defaults = lookup(Some(&rows), &cache, Some("proxy"));
    let found = defaults
        .default_end_user_budget(Some(""))
        .await
        .expect("the empty id is a real lookup");
    assert_eq!(
        found.map(|budget| budget.budget_id),
        Some(Some("from-empty".to_owned()))
    );
    assert_eq!(rows.queries(), vec![String::new()]);
    assert_eq!(
        cache.reads(),
        vec!["budget:default_end_user_budget:".to_owned()]
    );
}

#[tokio::test]
async fn no_db_or_id_skips_cache() {
    let cache = ScriptedCache::failing(Some(Surface::Budget), None);
    let without_database = lookup(None, &cache, Some("proxy"));
    assert!(
        without_database
            .default_end_user_budget(Some("b1"))
            .await
            .expect("an absent database answers None")
            .is_none()
    );
    assert!(cache.reads().is_empty());

    let rows = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let without_id = lookup(Some(&rows), &cache, None);
    assert!(
        without_id
            .default_end_user_budget(None)
            .await
            .expect("an unset id answers None")
            .is_none()
    );
    assert!(cache.reads().is_empty());
    assert!(rows.queries().is_empty());

    let team = lookup(None, &cache, Some("proxy"));
    assert!(
        team.team_member_default_budget("member-budget")
            .await
            .expect("an absent database skips the member cache")
            .is_none()
    );
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn default_cache_hit() {
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::QueryFailure);
    let cache = ScriptedCache::new();
    cache.put_budget(
        "default_end_user_budget:b1",
        budget_fixture("b1", Some("100")),
    );
    let found = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1"))
        .await
        .expect("the cache answers");
    assert_eq!(
        found.map(|budget| budget.budget_id),
        Some(Some("b1".to_owned()))
    );
    assert_eq!(
        cache.reads(),
        vec!["budget:default_end_user_budget:b1".to_owned()]
    );
    assert!(rows.queries().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn default_missing_warn() {
    let (log, _guard) = capture_events();
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1"))
        .await
        .expect("a missing row answers None");
    assert!(found.is_none());
    assert_eq!(rows.queries(), vec!["b1".to_owned()]);
    assert!(cache.writes().is_empty());
    assert_eq!(
        events_at(&log, Level::WARN),
        vec!["Default end user budget not found in database: b1".to_owned()]
    );
}

#[tokio::test]
async fn warning_id_strips_crlf() {
    let (log, _guard) = capture_events();
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1\r\nx"))
        .await
        .expect("a missing row answers None");
    assert!(found.is_none());
    assert_eq!(
        events_at(&log, Level::WARN),
        vec!["Default end user budget not found in database: b1x".to_owned()]
    );
}

#[tokio::test]
async fn default_query_error_none() {
    let (log, _guard) = capture_events();
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::QueryFailure);
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1"))
        .await
        .expect("a failed query answers None");
    assert!(found.is_none());
    assert_eq!(events_at(&log, Level::ERROR).len(), 1);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn default_decode_error_none() {
    let (log, _guard) = capture_events();
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::DecodeFailure);
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1"))
        .await
        .expect("an unreadable row answers None");
    assert!(found.is_none());
    assert_eq!(events_at(&log, Level::ERROR).len(), 1);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn default_cache_write_error_none() {
    let (log, _guard) = capture_events();
    let rows =
        ScriptedBudgetRows::answering(BudgetAnswer::Row(Some(budget_fixture("b1", Some("100")))));
    let cache = ScriptedCache::failing(None, Some(Surface::Budget));
    let found = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1"))
        .await
        .expect("a failed cache write answers None");
    assert!(found.is_none());
    assert_eq!(events_at(&log, Level::ERROR).len(), 1);
    assert_eq!(
        cache.writes(),
        vec!["budget:default_end_user_budget:b1:45".to_owned()]
    );
}

#[tokio::test]
async fn default_cache_read_error_propagates() {
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::failing(Some(Surface::Budget), None);
    let error = lookup(Some(&rows), &cache, None)
        .default_end_user_budget(Some("b1"))
        .await
        .expect_err("the cache read sits outside the catch");
    assert!(matches!(error, PayloadError::Json(_)));
    assert!(rows.queries().is_empty());
}

#[tokio::test]
async fn member_query_error_none() {
    let (log, _guard) = capture_events();
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::QueryFailure);
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, None)
        .team_member_default_budget("member")
        .await
        .expect("a failed member query answers None");
    assert!(found.is_none());
    assert_eq!(events_at(&log, Level::ERROR).len(), 1);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn member_missing_warn() {
    let (log, _guard) = capture_events();
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, None)
        .team_member_default_budget("member")
        .await
        .expect("a missing member budget answers None");
    assert!(found.is_none());
    assert_eq!(
        events_at(&log, Level::WARN),
        vec!["Team-default member budget not found in database: member".to_owned()]
    );
}

#[tokio::test]
async fn member_decode_error_propagates() {
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::DecodeFailure);
    let cache = ScriptedCache::new();
    let error = lookup(Some(&rows), &cache, None)
        .team_member_default_budget("member")
        .await
        .expect_err("the member decode sits outside the try block");
    assert!(matches!(error, MemberBudgetError::Decode(_)));
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn member_cache_write_error_propagates() {
    let rows =
        ScriptedBudgetRows::answering(BudgetAnswer::Row(Some(budget_fixture("member", None))));
    let cache = ScriptedCache::failing(None, Some(Surface::Budget));
    let error = lookup(Some(&rows), &cache, None)
        .team_member_default_budget("member")
        .await
        .expect_err("the member cache write escapes");
    assert!(matches!(error, MemberBudgetError::Cache(_)));
}

#[tokio::test]
async fn management_ttl_consumed() {
    let rows =
        ScriptedBudgetRows::answering(BudgetAnswer::Row(Some(budget_fixture("b1", Some("100")))));
    let cache = ScriptedCache::with_ttl(33.0_f64);
    let defaults = lookup(Some(&rows), &cache, None);
    assert!(
        defaults
            .default_end_user_budget(Some("b1"))
            .await
            .expect("the default reads")
            .is_some()
    );
    assert!(
        defaults
            .team_member_default_budget("member")
            .await
            .expect("the member read")
            .is_some()
    );
    assert_eq!(
        cache.writes(),
        vec![
            "budget:default_end_user_budget:b1:33".to_owned(),
            "budget:team_member_default_budget:member:33".to_owned(),
        ]
    );
}

#[tokio::test]
async fn key_before_proxy_default() {
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::QueryFailure);
    rows.set(
        "key",
        BudgetAnswer::Row(Some(budget_fixture("key-budget", Some("10")))),
    );
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, Some("proxy"))
        .resolve_default_end_user_budget(Some("key"))
        .await
        .expect("the key budget resolves");
    assert_eq!(
        found.map(|budget| budget.budget_id),
        Some(Some("key-budget".to_owned()))
    );
    assert_eq!(rows.queries(), vec!["key".to_owned()]);
    assert_eq!(
        cache.reads(),
        vec!["budget:default_end_user_budget:key".to_owned()]
    );
}

#[tokio::test]
async fn missing_key_budget_uses_proxy() {
    let rows = ScriptedBudgetRows::answering(BudgetAnswer::Row(Some(budget_fixture(
        "proxy-budget",
        Some("10"),
    ))));
    rows.set("key", BudgetAnswer::Row(None));
    let cache = ScriptedCache::new();
    let found = lookup(Some(&rows), &cache, Some("proxy"))
        .resolve_default_end_user_budget(Some("key"))
        .await
        .expect("the proxy default resolves");
    assert_eq!(
        found.map(|budget| budget.budget_id),
        Some(Some("proxy-budget".to_owned()))
    );
    assert_eq!(rows.queries(), vec!["key".to_owned(), "proxy".to_owned()]);
    assert_eq!(
        cache.reads(),
        vec![
            "budget:default_end_user_budget:key".to_owned(),
            "budget:default_end_user_budget:proxy".to_owned(),
        ]
    );
}
