#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::log_success_event;
use crate::model_budget_limiter::charging::{ChargeMetadata, PayloadIdentity, SuccessEvent};
use crate::model_budget_limiter::error::ChargeError;
use crate::model_budget_limiter::tests::support::{
    DAY, FailsAfterTwoCalls, FakeCache, amount, budgets, limiter, limiter_with,
};

const KEY_SPEND: &str = "virtual_key_spend:key-1:gpt-4o:1d";
const KEY_MARKER: &str = "virtual_key_spend:key-1:gpt-4o:1d:batch:batch-1";

fn metadata<'input>(
    key: Option<&'input BTreeMap<String, JsonData>>,
    team: Option<&'input BTreeMap<String, JsonData>>,
    user: Option<&'input BTreeMap<String, JsonData>>,
    end_user: Option<&'input BTreeMap<String, JsonData>>,
) -> ChargeMetadata<'input> {
    ChargeMetadata {
        key_max_budget: key,
        team_max_budget: team,
        user_max_budget: user,
        end_user_max_budget: end_user,
    }
}

fn identity<'input>(
    key_hash: Option<&'input str>,
    team_id: Option<&'input str>,
    user_id: Option<&'input str>,
    end_user: Option<&'input str>,
    end_user_id: Option<&'input str>,
) -> PayloadIdentity<'input> {
    PayloadIdentity {
        key_hash,
        team_id,
        user_id,
        end_user,
        end_user_id,
    }
}

fn key_budget() -> BTreeMap<String, JsonData> {
    budgets(r#"{"gpt-4o": {"max_budget": 10, "budget_duration": "1d"}}"#)
}

fn uncapped_key_budget() -> BTreeMap<String, JsonData> {
    budgets(r#"{"gpt-4o": {"budget_duration": "1d"}}"#)
}

#[tokio::test]
async fn charges_the_key_counter_under_the_configured_budget_model() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("openai/gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert_eq!(subject.cache.memory(KEY_SPEND), Some(amount("2")));
    let calls = subject.cache.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].duration, Duration::from_secs(DAY));
    assert_eq!(
        calls[0].start_time_key,
        "virtual_key_budget_start_time:key-1:gpt-4o:1d"
    );
    assert_eq!(calls[0].cost, amount("2"));
}

#[tokio::test]
async fn charges_the_team_user_and_end_user_scopes_beside_the_key() {
    let subject = limiter(FakeCache::new());
    let key = uncapped_key_budget();
    let team = key_budget();
    let user = key_budget();
    let end_user = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&key), Some(&team), Some(&user), Some(&end_user)),
        payload: identity(
            Some("key-1"),
            Some("team-1"),
            Some("user-1"),
            Some("end-user-1"),
            None,
        ),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    let calls = subject.cache.calls();
    let keys: Vec<&str> = calls.iter().map(|call| call.spend_key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "virtual_key_spend:key-1:gpt-4o:1d",
            "team_model_spend:team-1:gpt-4o:1d",
            "user_model_spend:user-1:gpt-4o:1d",
            "end_user_model_spend:end-user-1:gpt-4o:1d",
        ]
    );
}

#[tokio::test]
async fn the_team_counter_is_not_charged_when_the_key_gates_the_model() {
    let subject = limiter(FakeCache::new());
    let key = key_budget();
    let team = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&key), Some(&team), None, None),
        payload: identity(Some("key-1"), Some("team-1"), None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    let calls = subject.cache.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].spend_key, KEY_SPEND);
}

#[tokio::test]
async fn the_model_group_wins_over_the_deployment_model() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: Some("gpt-4o-mini"),
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert_eq!(subject.cache.memory(KEY_SPEND), Some(amount("2")));
}

#[tokio::test]
async fn the_deployment_model_is_used_without_a_model_group() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: None,
        model: Some("gpt-4o"),
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert_eq!(subject.cache.memory(KEY_SPEND), Some(amount("2")));
}

#[tokio::test]
async fn no_payload_charges_nothing() {
    let subject = limiter(FakeCache::new());
    log_success_event(&subject, None).await.unwrap();
    assert!(subject.cache.calls().is_empty());
}

#[tokio::test]
async fn no_model_charges_nothing() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: None,
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert!(subject.cache.calls().is_empty());
}

#[tokio::test]
async fn a_scope_without_an_entity_id_is_not_charged() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(None, None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert!(subject.cache.calls().is_empty());
}

#[tokio::test]
async fn an_empty_budget_map_is_not_charged() {
    let subject = limiter(FakeCache::new());
    let empty = budgets("{}");
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&empty), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert!(subject.cache.calls().is_empty());
}

#[tokio::test]
async fn a_model_no_budget_names_is_not_charged() {
    let subject = limiter(FakeCache::new());
    let budget = budgets(r#"{"gpt-4o-mini": {"max_budget": 10, "budget_duration": "1d"}}"#);
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert!(subject.cache.calls().is_empty());
}

#[tokio::test]
async fn a_malformed_entry_is_not_charged() {
    let subject = limiter(FakeCache::new());
    let budget = budgets(r#"{"gpt-4o": {"max_budget": 10}}"#);
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert!(subject.cache.calls().is_empty());
}

fn batch_event<'input>(
    budget: &'input BTreeMap<String, JsonData>,
    batch_id: &'input str,
    response_cost: Decimal,
) -> SuccessEvent<'input> {
    SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost,
        metadata: metadata(Some(budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: Some("retrieve_batch"),
        batch_id: Some(batch_id),
    }
}

#[tokio::test]
async fn a_retrieved_batch_is_charged_once_across_polls() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-1", amount("2"))),
    )
    .await
    .unwrap();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-1", amount("2"))),
    )
    .await
    .unwrap();
    assert_eq!(subject.cache.memory(KEY_SPEND), Some(amount("2")));
    assert_eq!(subject.cache.memory(KEY_MARKER), Some(amount("2")));
    assert_eq!(subject.cache.calls().len(), 1);
}

#[tokio::test]
async fn every_new_batch_is_charged_once() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-1", amount("2"))),
    )
    .await
    .unwrap();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-2", amount("3"))),
    )
    .await
    .unwrap();
    assert_eq!(subject.cache.memory(KEY_SPEND), Some(amount("5")));
    assert_eq!(subject.cache.calls().len(), 2);
}

#[tokio::test]
async fn a_zero_cost_batch_poll_is_charged_every_time() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-1", Decimal::ZERO)),
    )
    .await
    .unwrap();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-1", Decimal::ZERO)),
    )
    .await
    .unwrap();
    assert_eq!(subject.cache.calls().len(), 2);
}

#[tokio::test]
async fn a_batch_poll_that_is_not_a_retrieve_is_charged_every_time() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: Some("create_batch"),
        batch_id: Some("batch-1"),
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    log_success_event(&subject, Some(&event)).await.unwrap();
    assert_eq!(subject.cache.calls().len(), 2);
}

#[tokio::test]
async fn the_batch_marker_lives_for_the_budget_window() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    log_success_event(
        &subject,
        Some(&batch_event(&budget, "batch-1", amount("2"))),
    )
    .await
    .unwrap();
    let ttls = subject.cache.counter_ttls.lock().unwrap().clone();
    assert_eq!(ttls, [(KEY_MARKER.to_owned(), Duration::from_secs(DAY))]);
}

#[tokio::test]
async fn an_unreadable_charge_duration_is_an_error() {
    let subject = limiter_with(
        FakeCache::new(),
        Box::new(FailsAfterTwoCalls {
            calls: AtomicUsize::new(0),
        }),
    );
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    let error = log_success_event(&subject, Some(&event)).await.unwrap_err();
    assert!(matches!(error, ChargeError::Duration { .. }));
}

#[tokio::test]
async fn an_increment_failure_is_reported() {
    let cache = FakeCache::new();
    cache.fail_spend.store(true, Ordering::SeqCst);
    let subject = limiter(cache);
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    let error = log_success_event(&subject, Some(&event)).await.unwrap_err();
    assert!(matches!(error, ChargeError::Increment { .. }));
}

#[tokio::test]
async fn increments_are_flushed_only_when_redis_is_configured() {
    let without = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(Some(&budget), None, None, None),
        payload: identity(Some("key-1"), None, None, None, None),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&without, Some(&event)).await.unwrap();
    assert_eq!(without.cache.flushes.load(Ordering::SeqCst), 0);
    let with_redis = limiter(FakeCache::with_redis());
    log_success_event(&with_redis, Some(&event)).await.unwrap();
    assert_eq!(with_redis.cache.flushes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_logged_end_user_wins_over_the_metadata_end_user_id() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(None, None, None, Some(&budget)),
        payload: identity(None, None, None, Some("top-level"), Some("metadata")),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    let calls = subject.cache.calls();
    assert_eq!(
        calls[0].spend_key,
        "end_user_model_spend:top-level:gpt-4o:1d"
    );
}

#[tokio::test]
async fn the_metadata_end_user_id_is_used_without_a_logged_end_user() {
    let subject = limiter(FakeCache::new());
    let budget = key_budget();
    let event = SuccessEvent {
        model_group: Some("gpt-4o"),
        model: None,
        response_cost: amount("2"),
        metadata: metadata(None, None, None, Some(&budget)),
        payload: identity(None, None, None, None, Some("metadata")),
        call_type: None,
        batch_id: None,
    };
    log_success_event(&subject, Some(&event)).await.unwrap();
    let calls = subject.cache.calls();
    assert_eq!(
        calls[0].spend_key,
        "end_user_model_spend:metadata:gpt-4o:1d"
    );
}
