#![cfg(test)]

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::budget_permission_rows::JsonData;
use crate::database_faults::ErrorType;
use crate::identity_rows::key::Expiry;
use crate::key_hash::KeyHash;
use crate::key_user_writes::hash::hash_token;
use crate::team_rows::member::MemberRole;

use super::error::{KeyReadError, MISSING_TOKEN_DETAIL, MISSING_TOKEN_STATUS, ReconnectFailure};
use super::grace::{DEPRECATED_CACHE_CAPACITY, DEPRECATED_CACHE_TTL_SECONDS, DeprecatedKeyCache};
use super::request::{CombinedKeyRequest, KeyQueryType};
use super::rows::{CACHED_PLAN_WARNING, RECONNECT_REASON};
use super::view::{CombinedKeyView, epoch_decimal};

mod harness;
mod postgres;

use harness::{
    Harness, ObservedDiagnostic, RecordedRowCall, cached_plan_fault, decimal, events, fault,
    fixed_now, grace_hit, lock, number, object, text, view_fields, view_row,
};

#[tokio::test]
async fn token_hash_once_and_missing400() {
    let harness = Harness::new(Duration::ZERO);
    let plaintext = "sk-plaintext";
    harness.database.push_row(Ok(view_row(plaintext)));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some(plaintext)))
        .await
        .unwrap();
    assert!(found.is_some());
    let stored = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    harness.database.push_row(Ok(view_row(stored)));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some(stored)))
        .await
        .unwrap();
    assert!(found.is_some());
    let calls = harness.database.row_calls();
    assert_eq!(calls[0].hash, hash_token(plaintext));
    assert_eq!(calls[1].hash, stored);
    let error = harness
        .read(&CombinedKeyRequest::find_unique(None))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(MISSING_TOKEN_STATUS));
    assert_eq!(error.detail(), Some(MISSING_TOKEN_DETAIL));
    assert!(matches!(error, KeyReadError::MissingToken));
    assert_eq!(harness.database.row_calls().len(), 2);
}

#[tokio::test]
async fn non_unique_returns_none() {
    let harness = Harness::new(Duration::ZERO);
    let listed = CombinedKeyRequest {
        query_type: KeyQueryType::FindAll,
        ..CombinedKeyRequest::find_unique(Some("sk-listed"))
    };
    assert!(harness.read(&listed).await.unwrap().is_none());
    assert!(harness.database.row_calls().is_empty());
    assert!(harness.database.grace_calls().is_empty());
    let hashed = harness
        .diagnostics
        .seen()
        .into_iter()
        .filter(|diagnostic| matches!(diagnostic, ObservedDiagnostic::TokenHash))
        .count();
    assert_eq!(hashed, 1);
    let absent = CombinedKeyRequest {
        query_type: KeyQueryType::FindAll,
        ..CombinedKeyRequest::find_unique(None)
    };
    assert!(harness.read(&absent).await.unwrap().is_none());
    assert!(harness.database.row_calls().is_empty());
}

#[tokio::test]
async fn every_join_alias_and_null() {
    let fixture = postgres::Fixture::start().await;
    postgres::every_join_alias_and_null(&fixture).await;
    fixture.close().await;
}

#[tokio::test]
async fn membership_requires_both_ids() {
    let fixture = postgres::Fixture::start().await;
    postgres::membership_requires_both_ids(&fixture).await;
    fixture.close().await;
}

#[tokio::test]
async fn key_org_differs_from_team_org() {
    let fixture = postgres::Fixture::start().await;
    postgres::key_org_differs_from_team_org(&fixture).await;
    fixture.close().await;
}

#[tokio::test]
async fn writer_bypasses_plan_repair() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Ok(view_row("sk-writer")));
    let request = CombinedKeyRequest {
        use_writer: true,
        ..CombinedKeyRequest::find_unique(Some("sk-writer"))
    };
    assert!(harness.read(&request).await.unwrap().is_some());
    assert!(harness.reconnect.requests().is_empty());
    assert_eq!(
        harness.database.row_calls(),
        vec![RecordedRowCall {
            hash: hash_token("sk-writer"),
            use_writer: true,
        }]
    );
    assert!(!events(&harness.timeline).contains(&String::from("observe")));
}

#[tokio::test]
async fn writer_plan_error_is_not_repaired() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(cached_plan_fault()));
    harness.database.push_row(Err(cached_plan_fault()));
    harness.database.push_row(Err(cached_plan_fault()));
    let request = CombinedKeyRequest {
        use_writer: true,
        ..CombinedKeyRequest::find_unique(Some("sk-writer"))
    };
    let error = harness.read(&request).await.unwrap_err();
    assert!(matches!(error, KeyReadError::Database(_)));
    assert!(harness.reconnect.requests().is_empty());
    let calls = harness.database.row_calls();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| call.use_writer));
}

#[tokio::test]
async fn reader_generation_before_query() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(cached_plan_fault()));
    harness.database.push_row(Ok(view_row("sk-plan")));
    harness.reconnect.push_result(Ok(true));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-plan")))
        .await
        .unwrap();
    assert!(found.is_some());
    let timeline = events(&harness.timeline);
    let observed = position(&timeline, "observe");
    let queried = position(&timeline, "read");
    assert!(observed < queried);
    let requests = harness.reconnect.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].stale_is_live);
}

#[tokio::test]
async fn plan_error_exact_substring() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(fault(
        ErrorType::RawQuery,
        "error: cached plan must not change result type (copy 1)",
    )));
    harness.database.push_row(Ok(view_row("sk-plan")));
    harness.reconnect.push_result(Ok(true));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-plan")))
        .await
        .unwrap();
    assert!(found.is_some());
    assert!(harness.diagnostics.seen().iter().any(|diagnostic| {
        matches!(diagnostic, ObservedDiagnostic::CachedPlan(message) if message.as_str() == CACHED_PLAN_WARNING)
    }));
    assert_eq!(harness.reconnect.requests().len(), 1);
}

#[tokio::test]
async fn plan_error_case_variation_is_not_repaired() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(fault(
        ErrorType::RawQuery,
        "ERROR: Cached plan must not change result type",
    )));
    harness.database.push_row(Err(fault(
        ErrorType::RawQuery,
        "ERROR: Cached plan must not change result type",
    )));
    harness.database.push_row(Err(fault(
        ErrorType::RawQuery,
        "ERROR: Cached plan must not change result type",
    )));
    let error = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-plan")))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyReadError::Database(_)));
    assert!(harness.reconnect.requests().is_empty());
    assert_eq!(harness.database.row_calls().len(), 3);
}

#[tokio::test]
async fn same_sql_retry_after_false() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(cached_plan_fault()));
    harness.database.push_row(Ok(view_row("sk-retry")));
    harness.reconnect.push_result(Ok(false));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-retry")))
        .await
        .unwrap();
    assert!(found.is_some());
    let calls = harness.database.row_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], calls[1]);
    let reconnect = &harness.reconnect.requests()[0];
    assert_eq!(reconnect.reason, RECONNECT_REASON);
    assert!(!reconnect.force);
    assert!(reconnect.force_recreate);
    assert!(reconnect.timeout.is_none());
    assert!(reconnect.lock_timeout.is_none());
    assert!(!reconnect.has_database_url);
}

#[tokio::test]
async fn reconnect_throw_and_second_query_error() {
    reconnect_failure_stops_the_query().await;
    second_query_failure_is_returned().await;
}

#[tokio::test]
async fn ordinary_error_no_plan_retry() {
    let harness = failing_harness(Duration::ZERO, 3, "connection reset");
    let error = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-broken")))
        .await
        .unwrap_err();
    assert!(
        matches!(error, KeyReadError::Database(ref fault) if fault.message == "connection reset")
    );
    assert!(harness.reconnect.requests().is_empty());
    assert_eq!(harness.database.row_calls().len(), 3);
    assert_eq!(harness.pauses().len(), 2);
}

#[tokio::test]
async fn outer_three_attempts_ten_seconds_and_cancel() {
    three_attempts_with_two_waits().await;
    ten_seconds_stop_before_three_tries().await;
    wait_is_capped_by_remaining_time().await;
}

#[tokio::test]
async fn metrics_and_failure_hook_before_backoff() {
    let harness = failing_harness(Duration::from_millis(10), 3, "lookup failed");
    harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-metric")))
        .await
        .unwrap_err();
    let timeline = events(&harness.timeline);
    assert!(position(&timeline, "failure") < position(&timeline, "traceback"));
    assert!(position(&timeline, "traceback") < position(&timeline, "hook"));
    assert!(position(&timeline, "hook") < position(&timeline, "metric-failure"));
    assert!(position(&timeline, "metric-failure") < position(&timeline, "backoff"));
    assert!(position(&timeline, "backoff") < position(&timeline, "pause"));
    let hooks = harness.hook.hooks();
    assert_eq!(hooks[0].call_type, "get_data");
    assert_eq!(hooks[0].class, "database");
    assert!(hooks[0].message.contains("lookup failed"));
    assert!(hooks[0].duration > Duration::ZERO);
    let failures = lock(&harness.sink.failure_calls);
    assert_eq!(failures.len(), 3);
    assert_eq!(failures[0].0, "get_data");
}

#[test]
fn deprecated_ttl60_revoke_equality_and_capacity1000() {
    let now = fixed_now();
    let mut cache = DeprecatedKeyCache::default();
    let hash = KeyHash::from("cached-hash");
    cache.remember(
        hash.clone(),
        &grace_hit("active", now + TimeDelta::hours(1)),
        now,
    );
    assert_eq!(cache.lookup(&hash, now), Some(String::from("active")));
    let ttl = TimeDelta::seconds(DEPRECATED_CACHE_TTL_SECONDS);
    assert_eq!(
        cache.lookup(&hash, now + ttl - TimeDelta::nanoseconds(1)),
        Some(String::from("active"))
    );
    assert_eq!(cache.lookup(&hash, now + ttl), None);
    assert!(cache.entries.is_empty());
    let revoked = KeyHash::from("revoked");
    cache.remember(
        revoked.clone(),
        &grace_hit("active", now + TimeDelta::seconds(30)),
        now,
    );
    assert_eq!(cache.lookup(&revoked, now + TimeDelta::seconds(30)), None);
    let mut full = DeprecatedKeyCache::default();
    fill_cache(&mut full, DEPRECATED_CACHE_CAPACITY, now);
    assert_eq!(full.entries.len(), DEPRECATED_CACHE_CAPACITY);
    assert_eq!(full.lookup(&KeyHash::from("cache-key-0"), now), None);
    let newest = KeyHash::from(format!("cache-key-{DEPRECATED_CACHE_CAPACITY}"));
    assert_eq!(full.lookup(&newest, now), Some(String::from("active")));
    refresh_at_capacity(&mut full, now);
}

#[tokio::test]
async fn writer_bypasses_but_populates_grace_cache() {
    let harness = Harness::new(Duration::ZERO);
    let revoked = fixed_now() + TimeDelta::hours(1);
    harness.database.push_row(Ok(None));
    harness
        .database
        .push_grace(Ok(Some(grace_hit("sk-active", revoked))));
    harness.database.push_row(Ok(view_row("sk-active")));
    let request = CombinedKeyRequest {
        use_writer: true,
        ..CombinedKeyRequest::find_unique(Some("sk-old"))
    };
    assert!(harness.read(&request).await.unwrap().is_some());
    assert_eq!(harness.database.grace_calls().len(), 1);
    assert!(harness.database.grace_calls()[0].use_writer);
    assert_eq!(lock(&harness.cache).entries.len(), 1);
    harness.database.push_row(Ok(None));
    harness.database.push_row(Ok(view_row("sk-active")));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-old")))
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(harness.database.grace_calls().len(), 1);
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(None));
    assert!(harness.read(&request).await.unwrap().is_none());
    assert_eq!(harness.database.grace_calls().len(), 2);
}

#[tokio::test]
async fn negative_grace_not_cached() {
    let harness = Harness::new(Duration::ZERO);
    let request = CombinedKeyRequest::find_unique(Some("sk-miss"));
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(None));
    assert!(harness.read(&request).await.unwrap().is_none());
    assert!(lock(&harness.cache).entries.is_empty());
    harness.database.push_row(Ok(None));
    harness
        .database
        .push_grace(Ok(Some(grace_hit("", fixed_now() + TimeDelta::hours(1)))));
    assert!(harness.read(&request).await.unwrap().is_none());
    assert!(lock(&harness.cache).entries.is_empty());
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(None));
    assert!(harness.read(&request).await.unwrap().is_none());
    assert_eq!(harness.database.grace_calls().len(), 3);
}

#[tokio::test]
async fn grace_error_debug_drop() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Ok(None));
    harness
        .database
        .push_grace(Err(fault(ErrorType::Other, "deprecated table missing")));
    let found = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-err")))
        .await
        .unwrap();
    assert!(found.is_none());
    let expected = String::from("Deprecated key lookup skipped: deprecated table missing");
    assert!(
        harness
            .diagnostics
            .seen()
            .contains(&ObservedDiagnostic::DeprecatedLookupFailed(expected))
    );
    assert!(lock(&harness.cache).entries.is_empty());
}

#[tokio::test]
async fn grace_one_hop_typed_return() {
    grace_returns_the_active_view().await;
    grace_never_chains().await;
}

#[test]
fn last_matching_member_not_email() {
    let fields = view_fields(vec![
        ("user_id", text("u1")),
        (
            "team_members_with_roles",
            JsonData::Array(vec![
                object(vec![
                    ("user_id", text("u1")),
                    ("user_email", text("first@example.com")),
                    ("role", text("admin")),
                ]),
                object(vec![
                    ("user_id", JsonData::Null),
                    ("user_email", text("email-only@example.com")),
                    ("role", text("user")),
                ]),
                object(vec![
                    ("user_id", text("u1")),
                    ("user_email", text("last@example.com")),
                    ("role", text("user")),
                ]),
            ]),
        ),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    let member = view.team_member.unwrap();
    assert_eq!(member.base.user_email.as_deref(), Some("last@example.com"));
    assert_eq!(member.role, MemberRole::User);
}

#[test]
fn normalized_team_fields_and_expiry() {
    normalizes_team_defaults();
    applies_budget_aliases();
    stringifies_end_user_and_promotes_org();
    formats_expiry_like_isoformat();
}

#[tokio::test]
async fn native_database_diagnostics_hide_token_and_hash() {
    let harness = failing_harness(Duration::ZERO, 3, "the database is down");
    let token = "sk-super-secret-token";
    let error = harness
        .read(&CombinedKeyRequest::find_unique(Some(token)))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyReadError::Database(_)));
    let hash = hash_token(token);
    let messages: Vec<String> = harness
        .diagnostics
        .seen()
        .into_iter()
        .filter_map(|diagnostic| match diagnostic {
            ObservedDiagnostic::AttemptFailure(message)
            | ObservedDiagnostic::FailureTraceback(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 6);
    assert!(
        messages
            .iter()
            .all(|message| !message.contains(token) && !message.contains(&hash))
    );
    let hooks = harness.hook.hooks();
    assert!(hooks.iter().all(|hook| {
        !hook.message.contains(token)
            && !hook.message.contains(&hash)
            && !hook.traceback.contains(token)
            && !hook.traceback.contains(&hash)
    }));
    assert!(hooks[0].message.starts_with(
        "LiteLLM Prisma Client Exception: Error with `get_data`. Args passed in: [redacted] "
    ));
    assert!(hooks[0].message.ends_with("the database is down"));
}

fn position(timeline: &[String], label: &str) -> usize {
    timeline
        .iter()
        .position(|event| event == label)
        .unwrap_or(usize::MAX)
}

fn failing_harness(step: Duration, failures: usize, message: &str) -> Harness {
    let harness = Harness::new(step);
    for _ in 0..failures {
        harness
            .database
            .push_row(Err(fault(ErrorType::RawQuery, message)));
    }
    harness
}

async fn reconnect_failure_stops_the_query() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(cached_plan_fault()));
    harness.database.push_row(Err(cached_plan_fault()));
    harness.database.push_row(Err(cached_plan_fault()));
    harness.reconnect.push_result(Err(ReconnectFailure {
        message: String::from("reconnect exploded"),
    }));
    harness.reconnect.push_result(Err(ReconnectFailure {
        message: String::from("reconnect exploded"),
    }));
    harness.reconnect.push_result(Err(ReconnectFailure {
        message: String::from("reconnect exploded"),
    }));
    let error = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-throw")))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyReadError::Reconnect(_)));
    assert_eq!(harness.database.row_calls().len(), 3);
    assert_eq!(harness.reconnect.requests().len(), 3);
}

async fn second_query_failure_is_returned() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Err(cached_plan_fault()));
    harness
        .database
        .push_row(Err(fault(ErrorType::RawQuery, "second query failed")));
    harness.database.push_row(Err(cached_plan_fault()));
    harness
        .database
        .push_row(Err(fault(ErrorType::RawQuery, "second query failed")));
    harness.database.push_row(Err(cached_plan_fault()));
    harness
        .database
        .push_row(Err(fault(ErrorType::RawQuery, "second query failed")));
    harness.reconnect.push_result(Ok(true));
    harness.reconnect.push_result(Ok(true));
    harness.reconnect.push_result(Ok(true));
    let error = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-second")))
        .await
        .unwrap_err();
    assert!(
        matches!(error, KeyReadError::Database(ref fault) if fault.message == "second query failed")
    );
    assert_eq!(harness.database.row_calls().len(), 6);
}

async fn three_attempts_with_two_waits() {
    let harness = failing_harness(Duration::ZERO, 3, "lookup failed");
    harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-outer")))
        .await
        .unwrap_err();
    assert_eq!(harness.database.row_calls().len(), 3);
    assert_eq!(
        harness.pauses(),
        vec![Duration::from_secs(1), Duration::from_secs(2)]
    );
    let backoffs = harness
        .diagnostics
        .seen()
        .into_iter()
        .filter(|diagnostic| matches!(diagnostic, ObservedDiagnostic::Backoff(_)))
        .count();
    assert_eq!(backoffs, 2);
}

async fn ten_seconds_stop_before_three_tries() {
    let harness = failing_harness(Duration::from_secs(3), 3, "lookup failed");
    harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-outer")))
        .await
        .unwrap_err();
    assert_eq!(harness.database.row_calls().len(), 2);
    assert_eq!(harness.pauses().len(), 1);
}

async fn wait_is_capped_by_remaining_time() {
    let harness = failing_harness(Duration::from_millis(2250), 3, "lookup failed");
    harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-outer")))
        .await
        .unwrap_err();
    assert_eq!(harness.database.row_calls().len(), 3);
    assert_eq!(
        harness.pauses(),
        vec![Duration::from_secs(1), Duration::from_secs(1)]
    );
}

fn fill_cache(cache: &mut DeprecatedKeyCache, capacity: usize, now: DateTime<Utc>) {
    for index in 0..=capacity {
        let hash = KeyHash::from(format!("cache-key-{index}"));
        cache.remember(hash, &grace_hit("active", now + TimeDelta::hours(1)), now);
    }
}

fn refresh_at_capacity(cache: &mut DeprecatedKeyCache, now: DateTime<Utc>) {
    let newest = KeyHash::from(format!("cache-key-{DEPRECATED_CACHE_CAPACITY}"));
    cache.remember(
        newest.clone(),
        &grace_hit("active", now + TimeDelta::hours(1)),
        now,
    );
    assert_eq!(
        cache.entries.len(),
        DEPRECATED_CACHE_CAPACITY.saturating_sub(1)
    );
    assert_eq!(cache.lookup(&KeyHash::from("cache-key-1"), now), None);
    assert_eq!(cache.lookup(&newest, now), Some(String::from("active")));
}

async fn grace_returns_the_active_view() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(Some(grace_hit(
        "sk-active",
        fixed_now() + TimeDelta::hours(1),
    ))));
    harness.database.push_row(Ok(view_row("sk-active")));
    let view = harness
        .read(&CombinedKeyRequest::find_unique(Some("sk-old")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.token.token.unwrap().expose_secret(), "sk-active");
    assert!(
        harness
            .diagnostics
            .seen()
            .contains(&ObservedDiagnostic::DeprecatedUsed)
    );
    assert_eq!(harness.database.grace_calls().len(), 1);
}

async fn grace_never_chains() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(Some(grace_hit(
        "sk-gone",
        fixed_now() + TimeDelta::hours(1),
    ))));
    harness.database.push_row(Ok(None));
    assert!(
        harness
            .read(&CombinedKeyRequest::find_unique(Some("sk-old")))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(harness.database.grace_calls().len(), 1);
    assert!(
        !harness
            .diagnostics
            .seen()
            .contains(&ObservedDiagnostic::DeprecatedUsed)
    );
}

fn normalizes_team_defaults() {
    let fields = view_fields(vec![
        ("user_id", text("u1")),
        ("team_models", JsonData::Null),
        ("team_blocked", JsonData::Null),
        ("team_members_with_roles", JsonData::Array(Vec::new())),
    ]);
    let now = fixed_now();
    let view = CombinedKeyView::construct(fields, epoch_decimal(now)).unwrap();
    assert!(view.team_models.is_empty());
    assert!(!view.team_blocked);
    assert!(view.team_member.is_none());
    assert_eq!(view.last_refreshed_at, Some(epoch_decimal(now)));
}

fn applies_budget_aliases() {
    let fields = view_fields(vec![
        ("litellm_budget_table_max_budget", decimal("100")),
        ("litellm_budget_table_soft_budget", decimal("5")),
        ("litellm_budget_table_tpm_limit", number(21)),
        ("litellm_budget_table_rpm_limit", number(22)),
        ("litellm_budget_table_tpd_limit", number(23)),
        (
            "litellm_budget_table_model_max_budget",
            object(vec![("m", number(5))]),
        ),
        ("model_max_budget", object(Vec::new())),
        ("max_budget", decimal("7")),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(view.token.max_budget, Some(Decimal::from(7)));
    assert_eq!(view.soft_budget, Some(Decimal::from(5)));
    assert_eq!(view.token.tpm_limit, Some(21));
    assert_eq!(view.token.rpm_limit, Some(22));
    assert_eq!(view.token.tpd_limit, Some(23));
    assert_eq!(view.token.model_max_budget.0.get("m"), Some(&number(5)));
    let kept = view_fields(vec![
        (
            "litellm_budget_table_model_max_budget",
            object(vec![("b", number(1))]),
        ),
        ("model_max_budget", object(vec![("k", number(2))])),
    ]);
    let view = CombinedKeyView::construct(kept, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(view.token.model_max_budget.0.get("k"), Some(&number(2)));
    assert!(!view.token.model_max_budget.0.contains_key("b"));
}

fn stringifies_end_user_and_promotes_org() {
    let fields = view_fields(vec![
        ("end_user_id", number(42)),
        ("organization_id", text("org-key")),
        ("org_id", text("org-team")),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(view.end_user_id.as_deref(), Some("42"));
    assert_eq!(view.token.org_id.as_deref(), Some("org-key"));
    let flagged = view_fields(vec![("end_user_id", JsonData::Bool(true))]);
    let view = CombinedKeyView::construct(flagged, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(view.end_user_id.as_deref(), Some("True"));
}

fn formats_expiry_like_isoformat() {
    let micros = view_fields(vec![("expires", text("2024-01-02T03:04:05.120000"))]);
    let view = CombinedKeyView::construct(micros, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        view.token.expires,
        Some(Expiry::Text(String::from("2024-01-02T03:04:05.120000")))
    );
    let zero = view_fields(vec![("expires", text("2024-01-02T03:04:05.000000"))]);
    let view = CombinedKeyView::construct(zero, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        view.token.expires,
        Some(Expiry::Text(String::from("2024-01-02T03:04:05")))
    );
    let offset = view_fields(vec![("expires", text("2024-01-02T03:04:05+02:00"))]);
    let view = CombinedKeyView::construct(offset, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        view.token.expires,
        Some(Expiry::Text(String::from("2024-01-02T03:04:05+02:00")))
    );
}
