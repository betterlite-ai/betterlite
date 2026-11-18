use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::{JoinSet, yield_now};
use tokio::time::{advance, sleep};

use crate::routing_budget_state::event::{CallType, SuccessEvent};

use super::support::{
    DEPLOYMENT_KEY, PROVIDER_KEY, TAG_KEY, all_windows, cost, event, harness,
    harness_without_store, provider_without_a_period,
};

type CaseFuture = Pin<Box<dyn Future<Output = ()>>>;
type Case = fn() -> CaseFuture;

#[rustfmt::skip]
const CASES: &[(&str, Case)] = &[
    ("every_tier_spends_once", || Box::pin(every_tier_spends_once())),
    ("websocket_wrappers_spend_nothing", || Box::pin(websocket_wrappers_spend_nothing())),
    ("batch_call_types_still_spend", || Box::pin(batch_call_types_still_spend())),
    ("provider_without_a_period_spends_nothing", || Box::pin(provider_without_a_period_spends_nothing())),
    ("unconfigured_tiers_spend_nothing", || Box::pin(unconfigured_tiers_spend_nothing())),
    ("absent_model_id_names_no_deployment", || Box::pin(absent_model_id_names_no_deployment())),
    ("duplicate_tag_entries_spend_twice", || Box::pin(duplicate_tag_entries_spend_twice())),
    ("sync_flushes_then_copies", || Box::pin(sync_flushes_then_copies())),
    ("increments_during_the_flush_are_added_to_the_copy", || Box::pin(increments_during_the_flush_are_added_to_the_copy())),
    ("failed_flush_requeues_and_skips_the_copy", || Box::pin(failed_flush_requeues_and_skips_the_copy())),
    ("cancelled_sync_still_writes_the_flush", || Box::pin(cancelled_sync_still_writes_the_flush())),
    ("failed_read_keeps_the_local_spend", || Box::pin(failed_read_keeps_the_local_spend())),
    ("memory_only_sync_keeps_the_spend", || Box::pin(memory_only_sync_keeps_the_spend())),
    ("shutdown_flushes_the_queue", || Box::pin(shutdown_flushes_the_queue())),
    ("periodic_sync_copies_on_the_interval", || Box::pin(periodic_sync_copies_on_the_interval())),
    ("cancel_stops_the_periodic_sync", || Box::pin(cancel_stops_the_periodic_sync())),
];

pub(super) fn run(name: &str) -> CaseFuture {
    let Some((_, case)) = CASES.iter().find(|(case_name, _)| *case_name == name) else {
        panic!("unknown case {name}");
    };
    (*case)()
}

// async_log_success_event increments the provider, the deployment and each configured tag entry
async fn every_tier_spends_once() {
    let harness = harness(all_windows());
    let tags = ["team-a".to_owned(), "missing".to_owned()];
    harness
        .state
        .log_success(&event(Some("openai"), Some("dep-1"), &tags, cost("0.5")))
        .await
        .unwrap();
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    assert_eq!(harness.spend(DEPLOYMENT_KEY), Some(cost("0.5")));
    assert_eq!(harness.spend(TAG_KEY), Some(cost("0.5")));
    assert_eq!(
        harness.queued(),
        vec![
            (PROVIDER_KEY.to_owned(), cost("0.5")),
            (DEPLOYMENT_KEY.to_owned(), cost("0.5")),
            (TAG_KEY.to_owned(), cost("0.5")),
        ]
    );
}

// the wrapper call types return before any tier, because the turns inside them already spent
async fn websocket_wrappers_spend_nothing() {
    let harness = harness(all_windows());
    for call_type in ["_aresponses_websocket", "_arealtime"] {
        let wrapped = SuccessEvent {
            call_type: CallType::parse(Some(call_type)),
            ..event(Some("openai"), Some("dep-1"), &[], cost("0.5"))
        };
        harness.state.log_success(&wrapped).await.unwrap();
    }
    assert!(harness.queued().is_empty());
    assert_eq!(harness.spend(PROVIDER_KEY), None);
    assert_eq!(harness.spend(DEPLOYMENT_KEY), None);
}

// only those two wrappers are skipped: the batch call types the outcome counters skip still spend
async fn batch_call_types_still_spend() {
    let harness = harness(all_windows());
    for call_type in ["retrieve_batch", "aretrieve_batch"] {
        let batch = SuccessEvent {
            call_type: CallType::parse(Some(call_type)),
            ..event(Some("openai"), None, &[], cost("0.25"))
        };
        harness.state.log_success(&batch).await.unwrap();
    }
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    assert_eq!(harness.queued().len(), 2);
}

// _increment_spend_for_key returns before touching a window whose config has no duration
async fn provider_without_a_period_spends_nothing() {
    let harness = harness(provider_without_a_period());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    assert_eq!(harness.spend("provider_spend:openai:None"), None);
    assert!(harness.queued().is_empty());
    assert_eq!(harness.store().spend("provider_spend:openai:None"), None);
    assert_eq!(
        harness
            .store()
            .start_time("provider_budget_start_time:openai"),
        None
    );
    // the provider is configured, so a sync still reads its spend key back
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(
        harness.store().read_keys(),
        vec!["provider_spend:openai:None".to_owned()]
    );
}

// only a configured provider, deployment or tag opens a window
async fn unconfigured_tiers_spend_nothing() {
    let harness = harness(all_windows());
    let tags = ["missing".to_owned()];
    harness
        .state
        .log_success(&event(Some("anthropic"), Some("dep-9"), &tags, cost("0.5")))
        .await
        .unwrap();
    assert!(harness.queued().is_empty());
    assert_eq!(harness.spend(PROVIDER_KEY), None);
    assert_eq!(harness.spend(DEPLOYMENT_KEY), None);
}

// str(payload.get("model_id", "")): an event without an id names no deployment
async fn absent_model_id_names_no_deployment() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    assert_eq!(harness.spend(DEPLOYMENT_KEY), None);
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
}

// the tag tier loops over the request's entries, so a tag listed twice is charged twice
async fn duplicate_tag_entries_spend_twice() {
    let harness = harness(all_windows());
    let tags = ["team-a".to_owned(), "team-a".to_owned()];
    harness
        .state
        .log_success(&event(None, None, &tags, cost("0.25")))
        .await
        .unwrap();
    assert_eq!(harness.spend(TAG_KEY), Some(cost("0.5")));
    assert_eq!(harness.queued().len(), 2);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(TAG_KEY), Some(cost("0.5")));
}

// _sync_in_memory_spend_with_redis flushes this instance's spend, then copies the shared total back
async fn sync_flushes_then_copies() {
    let harness = harness(all_windows());
    harness.store().seed_spend(PROVIDER_KEY, cost("2"));
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("2.5")));
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("2.5")));
    assert!(harness.queued().is_empty());
}

// the copy adds what the queue still holds to the value it read, so the flush misses nothing
async fn increments_during_the_flush_are_added_to_the_copy() {
    let harness = harness(all_windows());
    harness.store().seed_spend(PROVIDER_KEY, cost("1"));
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.store().hold(true);
    let state = Arc::clone(&harness.state);
    let mut syncs = JoinSet::new();
    let _task = syncs.spawn(async move { state.sync_with_redis().await });
    harness.store().entered().notified().await;
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.25")))
        .await
        .unwrap();
    harness.store().hold(false);
    harness.store().release();
    let synced = syncs.join_next().await.unwrap().unwrap();
    assert!(synced);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("1.5")));
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("1.75")));
}

// a failed flush leaves the batch queued and returns before the copy
async fn failed_flush_requeues_and_skips_the_copy() {
    let harness = harness(all_windows());
    harness.store().fail(true);
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    assert!(!harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), None);
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    assert_eq!(
        harness.queued(),
        vec![(PROVIDER_KEY.to_owned(), cost("0.5"))]
    );
    harness.store().fail(false);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.5")));
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    assert!(harness.queued().is_empty());
}

// _await_flush_task: cancelling the sync does not cancel the write it started, so a held batch still lands
async fn cancelled_sync_still_writes_the_flush() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.store().hold(true);
    let state = Arc::clone(&harness.state);
    let mut syncs = JoinSet::new();
    let _task = syncs.spawn(async move { state.sync_with_redis().await });
    harness.store().entered().notified().await;
    syncs.abort_all();
    harness.store().hold(false);
    harness.store().release();
    let aborted = syncs.join_next().await.unwrap();
    aborted.unwrap_err();
    // the next flush waits behind the write the cancelled sync started, then finds the queue cleared
    let flushed = harness
        .state
        .increments
        .flush_increments(&harness.state.flushes)
        .await;
    assert!(flushed);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.5")));
    assert!(harness.queued().is_empty());
}

// a failed shared read copies nothing, so the local spend the increments built stays
async fn failed_read_keeps_the_local_spend() {
    let harness = harness(all_windows());
    harness.store().seed_spend(PROVIDER_KEY, cost("2"));
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.store().fail_reads(true);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("2.5")));
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    harness.store().fail_reads(false);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("2.5")));
}

// without a shared store there is nothing to copy, and a sync still drains the queue
async fn memory_only_sync_keeps_the_spend() {
    let harness = harness_without_store(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0.5")));
    assert!(harness.queued().is_empty());
}

// shutdown stops the periodic task, then flushes what the queue still holds
async fn shutdown_flushes_the_queue() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.state.shutdown().await.unwrap();
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.5")));
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.5")));
}

// periodic_sync_in_memory_spend_with_redis copies the shared spend again once the interval passes
async fn periodic_sync_copies_on_the_interval() {
    let harness = harness(all_windows());
    harness.store().seed_spend(PROVIDER_KEY, cost("3"));
    harness.state.start();
    sleep(Duration::from_millis(1)).await;
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("3")));
    harness.store().seed_spend(PROVIDER_KEY, cost("4"));
    advance(Duration::from_millis(998)).await;
    yield_now().await;
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("3")));
    advance(Duration::from_millis(1)).await;
    sleep(Duration::from_millis(2)).await;
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("4")));
}

// cancel_sync_task stops the loop, so a later shared write is never copied
async fn cancel_stops_the_periodic_sync() {
    let harness = harness(all_windows());
    harness.state.start();
    sleep(Duration::from_millis(1)).await;
    harness.state.cancel_sync_task();
    harness.store().seed_spend(PROVIDER_KEY, cost("3"));
    advance(Duration::from_secs(10)).await;
    sleep(Duration::from_millis(1)).await;
    assert_eq!(harness.spend(PROVIDER_KEY), None);
}
