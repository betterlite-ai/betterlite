use std::collections::{BTreeMap, HashMap};
use std::error::Error as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::thread;

use betterlite_redis::cache_primitives::decode::CachedValue;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::budget_increment::error::IncrementError;
use crate::budget_windows::service::BudgetWindows;
use crate::routing_budget_state::error::BudgetStateError;
use crate::routing_budget_state::spend_redis::{
    SpendRead, decode_spends, set_local_spend, spend_keys,
};

use super::support::{
    DEPLOYMENT_KEY, PROVIDER_KEY, TAG_KEY, TestClock, all_windows, cost, deployment, event,
    harness, harness_at, harness_without_store, providers, spend_error, tags,
};

type CaseFuture = Pin<Box<dyn Future<Output = ()>>>;
type Case = fn() -> CaseFuture;

#[rustfmt::skip]
const CASES: &[(&str, Case)] = &[
    ("zero_cost_still_opens_the_window", || Box::pin(zero_cost_still_opens_the_window())),
    ("spend_keys_cover_every_tier_and_the_sync_reads_them", || Box::pin(spend_keys_cover_every_tier_and_the_sync_reads_them())),
    ("shared_spend_read_decodes_values", || Box::pin(async { shared_spend_read_decodes_values() })),
    ("shared_spend_read_fails_on_unreadable_replies", || Box::pin(async { shared_spend_read_fails_on_unreadable_replies() })),
    ("copy_keeps_a_live_expiry_and_replaces_an_expired_one", || Box::pin(copy_keeps_a_live_expiry_and_replaces_an_expired_one())),
    ("repeated_failures_merge_the_batch_once", || Box::pin(repeated_failures_merge_the_batch_once())),
    ("increment_error_reports_the_key_and_source", || Box::pin(increment_error_reports_the_key_and_source())),
    ("invalid_clock_reports_the_key", || Box::pin(invalid_clock_reports_the_key())),
    ("start_without_a_runtime_starts_no_task", start_without_a_runtime),
    ("sync_without_a_store_returns_true", || Box::pin(sync_without_a_store_returns_true())),
    ("second_shutdown_flushes_new_increments", || Box::pin(second_shutdown_flushes_new_increments())),
    ("shutdown_reports_the_entries_it_could_not_write", || Box::pin(shutdown_reports_the_entries_it_could_not_write())),
];

pub(super) fn run(name: &str) -> CaseFuture {
    let Some((_, case)) = CASES.iter().find(|(case_name, _)| *case_name == name) else {
        panic!("unknown case {name}");
    };
    (*case)()
}

// a zero cost is still a spend: the window opens and the increment reaches the shared store
async fn zero_cost_still_opens_the_window() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0")))
        .await
        .unwrap();
    assert_eq!(harness.spend(PROVIDER_KEY), Some(cost("0")));
    assert_eq!(harness.queued(), vec![(PROVIDER_KEY.to_owned(), cost("0"))]);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0")));
}

// every configured window is read, keyed by its configured duration text, even one without a duration
async fn spend_keys_cover_every_tier_and_the_sync_reads_them() {
    let windows = BudgetWindows::new(Some(providers(None)), Some(tags()), &[deployment()]);
    let expected = vec![
        "provider_spend:openai:None".to_owned(),
        DEPLOYMENT_KEY.to_owned(),
        TAG_KEY.to_owned(),
    ];
    assert_eq!(spend_keys(&windows), expected);
    assert!(spend_keys(&BudgetWindows::default()).is_empty());
    let harness = harness(windows);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().read_keys(), expected);
}

// a numeric or text reply decodes, and a null reply is no spend
fn shared_spend_read_decodes_values() {
    let keys = vec![
        PROVIDER_KEY.to_owned(),
        TAG_KEY.to_owned(),
        DEPLOYMENT_KEY.to_owned(),
    ];
    let values = HashMap::from([
        (
            PROVIDER_KEY.to_owned(),
            Some(CachedValue::Number(Number::from_f64(2.5).unwrap())),
        ),
        (
            TAG_KEY.to_owned(),
            Some(CachedValue::Text("3.75".to_owned())),
        ),
    ]);
    let SpendRead::Values(spends) = decode_spends(&keys, &values) else {
        panic!("a numeric reply decodes");
    };
    assert_eq!(spends.get(PROVIDER_KEY), Some(&Some(cost("2.5"))));
    assert_eq!(spends.get(TAG_KEY), Some(&Some(cost("3.75"))));
    assert_eq!(spends.get(DEPLOYMENT_KEY), Some(&None));
    let nulls = HashMap::from([(PROVIDER_KEY.to_owned(), None)]);
    let SpendRead::Values(null_spends) = decode_spends(&keys, &nulls) else {
        panic!("a null reply decodes to no spend");
    };
    assert_eq!(null_spends.get(PROVIDER_KEY), Some(&None));
}

// a shape float() cannot read fails the batch, and an empty reply to a non-empty request fails it too
fn shared_spend_read_fails_on_unreadable_replies() {
    let keys = vec![
        PROVIDER_KEY.to_owned(),
        TAG_KEY.to_owned(),
        DEPLOYMENT_KEY.to_owned(),
    ];
    for unreadable in [
        CachedValue::Boolean(true),
        CachedValue::Text("not money".to_owned()),
        CachedValue::Array(Vec::new()),
    ] {
        let reply = HashMap::from([(PROVIDER_KEY.to_owned(), Some(unreadable))]);
        assert_eq!(decode_spends(&keys, &reply), SpendRead::Failed);
    }
    assert_eq!(decode_spends(&keys, &HashMap::new()), SpendRead::Failed);
    // an empty request reads nothing
    assert_eq!(
        decode_spends(&[], &HashMap::new()),
        SpendRead::Values(BTreeMap::new())
    );
}

// async_set_cache keeps a live expiry and gives an entry that already expired the cache default
async fn copy_keeps_a_live_expiry_and_replaces_an_expired_one() {
    let clock = TestClock::new(0.0);
    let harness = harness_at(all_windows(), &clock);
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    assert_eq!(harness.expiry(PROVIDER_KEY), Some(86_400.0_f64));
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.expiry(PROVIDER_KEY), Some(86_400.0_f64));
    clock.set(90_000.0);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.expiry(PROVIDER_KEY), Some(90_600.0_f64));
}

// each failed flush requeues its batch, and the retry writes the merged total exactly once
async fn repeated_failures_merge_the_batch_once() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.store().fail(true);
    assert!(!harness.state.sync_with_redis().await);
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.25")))
        .await
        .unwrap();
    assert!(!harness.state.sync_with_redis().await);
    assert_eq!(
        harness.queued(),
        vec![(PROVIDER_KEY.to_owned(), cost("0.75"))]
    );
    harness.store().fail(false);
    assert!(harness.state.sync_with_redis().await);
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.75")));
}

// a failed increment names the spend key and carries the window error that stopped it
async fn increment_error_reports_the_key_and_source() {
    let harness = harness(all_windows());
    set_local_spend(
        harness.state.increments.cache.as_ref(),
        PROVIDER_KEY,
        Decimal::MAX,
    )
    .unwrap();
    let error = spend_error(&harness).await;
    assert_eq!(
        error.to_string(),
        format!("the budget spend increment for {PROVIDER_KEY} failed")
    );
    let BudgetStateError::Increment { source, .. } = error else {
        panic!("the spend overflowed");
    };
    assert!(matches!(source, IncrementError::SpendOverflow { .. }));
}

// a clock that is not a finite number fails before any window is read
async fn invalid_clock_reports_the_key() {
    let harness = harness_at(all_windows(), &TestClock::new(f64::NAN));
    let error = spend_error(&harness).await;
    assert_eq!(
        error.to_string(),
        format!("the budget spend increment for {PROVIDER_KEY} failed")
    );
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<IncrementError>())
        .expect("the increment kept its source");
    assert!(matches!(source, IncrementError::InvalidSeconds { .. }));
}

// a thread without a runtime has no handle, so start leaves no task behind instead of panicking
fn start_without_a_runtime() -> CaseFuture {
    let harness = harness(all_windows());
    let state = Arc::clone(&harness.state);
    thread::scope(|scope| {
        let _handle = scope.spawn(move || state.start());
    });
    assert_eq!(harness.state.sync.lock().unwrap().len(), 0);
    Box::pin(async {})
}

// with no shared store the flush has nothing to write, and the sync still reports success
async fn sync_without_a_store_returns_true() {
    let harness = harness_without_store(all_windows());
    assert!(harness.state.sync_with_redis().await);
}

// shutdown is final for the loop but not for the queue: a later shutdown flushes what arrived after
async fn second_shutdown_flushes_new_increments() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.state.shutdown().await.unwrap();
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.5")));
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.25")))
        .await
        .unwrap();
    harness.state.shutdown().await.unwrap();
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.75")));
}

// a store that keeps rejecting the flush makes shutdown fail with the queue it could not write
async fn shutdown_reports_the_entries_it_could_not_write() {
    let harness = harness(all_windows());
    harness
        .state
        .log_success(&event(Some("openai"), None, &[], cost("0.5")))
        .await
        .unwrap();
    harness.store().fail(true);
    let error = harness.state.shutdown().await.unwrap_err();
    let BudgetStateError::Shutdown { entries } = error else {
        panic!("the store rejected the flush");
    };
    assert_eq!(entries, 1);
    assert_eq!(
        harness.queued(),
        vec![(PROVIDER_KEY.to_owned(), cost("0.5"))]
    );
    harness.store().fail(false);
    harness.state.shutdown().await.unwrap();
    assert_eq!(harness.store().spend(PROVIDER_KEY), Some(cost("0.5")));
}
