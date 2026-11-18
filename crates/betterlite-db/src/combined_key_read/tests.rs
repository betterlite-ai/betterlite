#![cfg(test)]

use std::error::Error as _;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::JsonData;
use crate::database_failures::metrics::{ProxySpanResolver, WallClock};
use crate::database_faults::ErrorType;
use crate::identity_rows::key::Expiry;
use crate::key_hash::KeyHash;
use crate::key_user_writes::hash::hash_token;
use crate::team_rows::member::MemberRole;

use super::error::{
    DatabaseFailure, KeyReadError, MISSING_TOKEN_DETAIL, MISSING_TOKEN_STATUS, ReconnectFailure,
};
use super::grace::{
    DEPRECATED_CACHE_CAPACITY, DEPRECATED_CACHE_TTL_SECONDS, DeprecatedKeyCache, DeprecatedToken,
};
use super::ports::{MetricWallClock, NoSpanResolver};
use super::request::{CombinedKeyRequest, KeyQueryType};
use super::rows::{CACHED_PLAN_WARNING, RECONNECT_REASON};
use super::view::{CombinedKeyView, ViewError, epoch_decimal};

mod harness;
mod postgres;

use harness::{
    Harness, ObservedDiagnostic, RecordedRowCall, StepClock, cached_plan_fault, decimal, events,
    fault, fixed_now, grace_hit, lock, number, object, text, view_fields, view_row,
};

#[tokio::test]
async fn token_hash_once_and_missing400() {
    let harness = Harness::new(Duration::ZERO);
    let plaintext = "sk-plaintext";
    harness.database.push_row(Ok(view_row(plaintext)));
    let hashed_request = CombinedKeyRequest {
        token: Some(SecretString::from(plaintext)),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let found = harness.read(&hashed_request).await.unwrap();
    assert!(found.is_some());
    let stored = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    harness.database.push_row(Ok(view_row(stored)));
    let stored_request = CombinedKeyRequest {
        token: Some(SecretString::from(stored)),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let found = harness.read(&stored_request).await.unwrap();
    assert!(found.is_some());
    let calls = harness.database.row_calls();
    assert_eq!(calls[0].hash, hash_token(plaintext));
    assert_eq!(calls[1].hash, stored);
    let absent_request = CombinedKeyRequest {
        token: None,
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let error = harness.read(&absent_request).await.unwrap_err();
    assert_eq!(error.status_code(), Some(MISSING_TOKEN_STATUS));
    assert_eq!(error.detail(), Some(MISSING_TOKEN_DETAIL));
    assert!(matches!(error, KeyReadError::MissingToken));
    assert_eq!(harness.database.row_calls().len(), 2);
}

#[tokio::test]
async fn non_unique_returns_none() {
    let harness = Harness::new(Duration::ZERO);
    let listed = CombinedKeyRequest {
        token: Some(SecretString::from("sk-listed")),
        query_type: KeyQueryType::FindAll,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
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
        token: None,
        query_type: KeyQueryType::FindAll,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
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
        token: Some(SecretString::from("sk-writer")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: true,
        parent_otel_span: None,
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
        token: Some(SecretString::from("sk-writer")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: true,
        parent_otel_span: None,
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-plan")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let found = harness.read(&request).await.unwrap();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-plan")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let found = harness.read(&request).await.unwrap();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-plan")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let error = harness.read(&request).await.unwrap_err();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-retry")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let found = harness.read(&request).await.unwrap();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-broken")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let error = harness.read(&request).await.unwrap_err();
    assert!(matches!(
        error,
        KeyReadError::Database(DatabaseFailure::Reported(ref fault))
            if fault.message == "connection reset"
    ));
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-metric")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    harness.read(&request).await.unwrap_err();
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

#[tokio::test]
async fn failure_hook_carries_the_original_failure() {
    let harness = failing_harness(Duration::ZERO, 3, "the database is down");
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-typed")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let returned = harness.read(&request).await.unwrap_err();
    let hooks = harness.hook.hooks();
    assert_eq!(hooks.len(), 3);
    assert_eq!(hooks[0].failure.class(), "database");
    assert_eq!(hooks[0].failure.status_code(), None);
    assert_eq!(hooks[0].failure.to_string(), returned.to_string());
    match &hooks[0].failure {
        KeyReadError::Database(DatabaseFailure::Reported(fault)) => {
            assert_eq!(fault.error_type, ErrorType::RawQuery);
            assert_eq!(fault.message, "the database is down");
            assert!(fault.code.is_none());
        }
        other => panic!("expected the reported database failure, got {other:?}"),
    }
    let malformed = Harness::new(Duration::ZERO);
    for _ in 0..3 {
        malformed
            .database
            .push_row(Ok(Some(JsonData::Object(view_fields(vec![(
                "end_user_id",
                JsonData::Number(serde_json::Number::from_f64(1.5).unwrap()),
            )])))));
    }
    let malformed_request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-typed")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    malformed.read(&malformed_request).await.unwrap_err();
    let hooks = malformed.hook.hooks();
    assert_eq!(hooks[0].failure.class(), "view");
    assert!(matches!(
        &hooks[0].failure,
        KeyReadError::View(ViewError::Json(_))
    ));
    assert!(hooks[0].failure.source().is_some());
}

#[tokio::test]
async fn failure_traceback_keeps_the_cause_chain() {
    let harness = Harness::new(Duration::ZERO);
    let cause = fault(ErrorType::RawQuery, "the connection dropped");
    let mut outer = fault(ErrorType::RawQuery, "the lookup failed");
    outer.cause = Some(Box::new(cause));
    for _ in 0..3 {
        harness.database.push_row(Err(outer.clone()));
    }
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-cause")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let returned = harness.read(&request).await.unwrap_err();
    let hooks = harness.hook.hooks();
    assert!(hooks[0].message.ends_with("the lookup failed"));
    assert!(!hooks[0].message.contains("the connection dropped"));
    assert!(
        hooks[0]
            .traceback
            .contains("Caused by: the connection dropped")
    );
    assert!(returned.source().is_some());
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
    let cached = cache.lookup(&hash, now).unwrap();
    assert_eq!(cached.expose_secret(), "active");
    let ttl = TimeDelta::seconds(DEPRECATED_CACHE_TTL_SECONDS);
    let almost = cache
        .lookup(&hash, now + ttl - TimeDelta::nanoseconds(1))
        .unwrap();
    assert_eq!(almost.expose_secret(), "active");
    assert!(cache.lookup(&hash, now + ttl).is_none());
    assert!(cache.entries.is_empty());
    let revoked = KeyHash::from("revoked");
    cache.remember(
        revoked.clone(),
        &grace_hit("active", now + TimeDelta::seconds(30)),
        now,
    );
    assert!(
        cache
            .lookup(&revoked, now + TimeDelta::seconds(30))
            .is_none()
    );
    let mut full = DeprecatedKeyCache::default();
    fill_cache(&mut full, DEPRECATED_CACHE_CAPACITY, now);
    assert_eq!(full.entries.len(), DEPRECATED_CACHE_CAPACITY);
    assert!(full.lookup(&KeyHash::from("cache-key-0"), now).is_none());
    let newest = KeyHash::from(format!("cache-key-{DEPRECATED_CACHE_CAPACITY}"));
    let stored = full.lookup(&newest, now).unwrap();
    assert_eq!(stored.expose_secret(), "active");
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
    let writer_request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-old")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: true,
        parent_otel_span: None,
    };
    let reader_request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-old")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    assert!(harness.read(&writer_request).await.unwrap().is_some());
    assert_eq!(harness.database.grace_calls().len(), 1);
    assert!(harness.database.grace_calls()[0].use_writer);
    assert_eq!(lock(&harness.cache).entries.len(), 1);
    harness.database.push_row(Ok(None));
    harness.database.push_row(Ok(view_row("sk-active")));
    let found = harness.read(&reader_request).await.unwrap();
    assert!(found.is_some());
    assert_eq!(harness.database.grace_calls().len(), 1);
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(None));
    assert!(harness.read(&writer_request).await.unwrap().is_none());
    assert_eq!(harness.database.grace_calls().len(), 2);
}

#[tokio::test]
async fn negative_grace_not_cached() {
    let harness = Harness::new(Duration::ZERO);
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-miss")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-err")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let found = harness.read(&request).await.unwrap();
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
                object(vec![
                    ("user_id", text("u2")),
                    ("user_email", text("other@example.com")),
                    ("role", text("admin")),
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from(token)),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let error = harness.read(&request).await.unwrap_err();
    assert!(matches!(error, KeyReadError::Database(_)));
    let hash = hash_token(token);
    let messages: Vec<String> = harness
        .diagnostics
        .seen()
        .into_iter()
        .filter_map(|diagnostic| match diagnostic {
            ObservedDiagnostic::AttemptFailure(message)
            | ObservedDiagnostic::FailureTraceback(message) => Some(message),
            ObservedDiagnostic::TokenHash
            | ObservedDiagnostic::CachedPlan(_)
            | ObservedDiagnostic::Backoff(_)
            | ObservedDiagnostic::DeprecatedUsed
            | ObservedDiagnostic::DeprecatedLookupFailed(_) => None,
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
    assert_eq!(hooks[0].traceback, hooks[0].message);
    assert!(!format!("{:?}", hooks[0].failure).contains(token));
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-throw")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let error = harness.read(&request).await.unwrap_err();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-second")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let error = harness.read(&request).await.unwrap_err();
    assert!(matches!(
        error,
        KeyReadError::Database(DatabaseFailure::Reported(ref fault))
            if fault.message == "second query failed"
    ));
    assert_eq!(harness.database.row_calls().len(), 6);
}

async fn three_attempts_with_two_waits() {
    let harness = failing_harness(Duration::ZERO, 3, "lookup failed");
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-outer")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    harness.read(&request).await.unwrap_err();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-outer")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    harness.read(&request).await.unwrap_err();
    assert_eq!(harness.database.row_calls().len(), 2);
    assert_eq!(harness.pauses().len(), 1);
}

async fn wait_is_capped_by_remaining_time() {
    let harness = failing_harness(Duration::from_millis(2250), 3, "lookup failed");
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-outer")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    harness.read(&request).await.unwrap_err();
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
    assert!(cache.lookup(&KeyHash::from("cache-key-1"), now).is_none());
    let stored = cache.lookup(&newest, now).unwrap();
    assert_eq!(stored.expose_secret(), "active");
}

async fn grace_returns_the_active_view() {
    let harness = Harness::new(Duration::ZERO);
    harness.database.push_row(Ok(None));
    harness.database.push_grace(Ok(Some(grace_hit(
        "sk-active",
        fixed_now() + TimeDelta::hours(1),
    ))));
    harness.database.push_row(Ok(view_row("sk-active")));
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-old")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    let view = harness.read(&request).await.unwrap().unwrap();
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
    let request = CombinedKeyRequest {
        token: Some(SecretString::from("sk-old")),
        query_type: KeyQueryType::FindUnique,
        check_deprecated: true,
        use_writer: false,
        parent_otel_span: None,
    };
    assert!(harness.read(&request).await.unwrap().is_none());
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

#[test]
fn epoch_decimal_whole_and_fractional_seconds() {
    assert_eq!(epoch_decimal(fixed_now()), Decimal::from(1_700_000_000_i64));
    let fractional = DateTime::from_timestamp(1_700_000_000, 250_000_000).unwrap();
    assert_eq!(epoch_decimal(fractional), Decimal::new(1_700_000_000_25, 2));
}

#[test]
fn null_budget_alias_never_clears_the_key_mapping() {
    let fields = view_fields(vec![
        ("model_max_budget", object(Vec::new())),
        ("litellm_budget_table_model_max_budget", JsonData::Null),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert!(view.token.model_max_budget.0.is_empty());
}

#[test]
fn fractional_end_user_id_is_a_typed_error() {
    let fields = view_fields(vec![(
        "end_user_id",
        JsonData::Number(serde_json::Number::from_f64(1.5).unwrap()),
    )]);
    assert!(matches!(
        CombinedKeyView::construct(fields, epoch_decimal(fixed_now())),
        Err(ViewError::Json(_))
    ));
}

#[test]
fn deprecated_token_columns_require_id_and_revoke_at() {
    let revoke_at = fixed_now().naive_utc();
    let token =
        DeprecatedToken::from_columns(Some(String::from("active-id")), Some(revoke_at)).unwrap();
    assert_eq!(token.active_token_id.expose_secret(), "active-id");
    assert_eq!(token.revoke_at, fixed_now());
    assert_eq!(
        DeprecatedToken::from_columns(None, Some(revoke_at))
            .unwrap_err()
            .to_string(),
        "the deprecated token row has no active_token_id"
    );
    assert_eq!(
        DeprecatedToken::from_columns(Some(String::from("active-id")), None)
            .unwrap_err()
            .to_string(),
        "the deprecated token row has no revoke_at"
    );
}

#[test]
fn metric_clock_reads_the_key_read_clock() {
    let clock = StepClock::new(Duration::ZERO, fixed_now());
    assert_eq!(MetricWallClock(&clock).now(), fixed_now());
    clock.advance_wall(Duration::from_secs(5));
    assert_eq!(
        MetricWallClock(&clock).now(),
        fixed_now() + TimeDelta::seconds(5)
    );
}

#[test]
fn no_span_resolver_never_invents_a_parent_span() {
    assert!(NoSpanResolver.parent_span(&IndexMap::new()).is_none());
}

#[tokio::test]
async fn native_driver_error_message_is_classified() {
    let fixture = postgres::Fixture::start().await;
    postgres::native_driver_error_message_is_classified(&fixture).await;
    fixture.close().await;
}

#[tokio::test]
async fn ordinary_driver_error_is_not_a_cached_plan() {
    let fixture = postgres::Fixture::start().await;
    postgres::ordinary_driver_error_is_not_a_cached_plan(&fixture).await;
    fixture.close().await;
}

#[tokio::test]
async fn deprecated_rotation_reads_the_active_key() {
    let fixture = postgres::Fixture::start().await;
    postgres::deprecated_rotation_reads_the_active_key(&fixture).await;
    fixture.close().await;
}
