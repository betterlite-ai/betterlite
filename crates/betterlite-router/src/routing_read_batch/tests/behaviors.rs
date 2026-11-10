use std::collections::BTreeMap;
use std::future::Future as _;
use std::slice;
use std::task::{Context, Waker};

use super::support::{
    HangingStore, MemoryStore, TestClock, counter, key, keys, read_keys, read_once, seed, tier,
    tiers, usage, values,
};
use crate::routing_read_batch::store::ReadWarning;
use crate::routing_read_batch::{
    PrefetchedUsage, ReadTiers, RequestReads, RoutingPrefetch, RoutingReadBatch,
    RoutingReadResults, RoutingReadSession, UsageReadKeys,
};
use crate::routing_value_store::RouteValue;

pub(crate) async fn run(name: &str) {
    match name {
        "arm_without_a_remote_store" => arm_without_a_remote_store(),
        "arm_keeps_the_first_prefetch" => arm_keeps_the_first_prefetch(),
        "arm_reserves_only_missing_keys" => arm_reserves_only_missing_keys(),
        "arm_merges_cooldown_and_usage_keys" => arm_merges_cooldown_and_usage_keys(),
        "arm_with_nothing_due" => arm_with_nothing_due(),
        "read_uses_the_prefetch_once" => read_uses_the_prefetch_once().await,
        "read_falls_back_after_a_failed_prefetch" => {
            read_falls_back_after_a_failed_prefetch().await;
        }
        "read_falls_back_when_the_prefetch_is_short" => {
            read_falls_back_when_the_prefetch_is_short().await;
        }
        "backfill_keeps_local_values" => backfill_keeps_local_values().await,
        "read_merges_the_usage_counters" => read_merges_the_usage_counters().await,
        "the_prefetched_usage_serves_the_next_read" => {
            the_prefetched_usage_serves_the_next_read().await;
        }
        "read_without_a_selector_keeps_no_prefetched_usage" => {
            read_without_a_selector_keeps_no_prefetched_usage().await;
        }
        "requests_are_isolated" => requests_are_isolated().await,
        "cancelled_read_releases_the_reservation" => cancelled_read_releases_the_reservation(),
        "prefetched_usage_covers_the_strategy_keys" => prefetched_usage_covers_the_strategy_keys(),
        "prefetched_usage_values_are_none_when_failed" => {
            prefetched_usage_values_are_none_when_failed();
        }
        "for_strategy_uses_usage_keys_without_a_prefetch" => {
            for_strategy_uses_usage_keys_without_a_prefetch();
        }
        "for_strategy_needs_an_armed_prefetch_without_usage" => {
            for_strategy_needs_an_armed_prefetch_without_usage();
        }
        "for_strategy_keeps_no_usage_keys" => for_strategy_keeps_no_usage_keys(),
        _ => panic!("unknown case {name}"),
    }
}

fn arm_without_a_remote_store() {
    let clock = TestClock::new();
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    let armed = RoutingPrefetch::arm(
        &read_keys(vec![key("a")], None),
        &tiers,
        false,
        &mut request,
    );
    assert!(!armed);
    assert!(request.prefetch.is_none());
}

fn arm_keeps_the_first_prefetch() {
    let clock = TestClock::new();
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    let first = RoutingPrefetch::arm(&read_keys(vec![key("a")], None), &tiers, true, &mut request);
    let second = RoutingPrefetch::arm(&read_keys(vec![key("b")], None), &tiers, true, &mut request);
    assert!(first);
    assert!(!second);
    let prefetch = request.prefetch.as_ref().unwrap();
    assert!(prefetch.keys.contains(&key("a")));
    assert!(!prefetch.keys.contains(&key("b")));
    assert_eq!(prefetch.fetched, vec![key("a")]);
}

fn arm_reserves_only_missing_keys() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let tiers = tiers(cooldown, None);
    let mut request = RequestReads::default();
    let armed = RoutingPrefetch::arm(
        &read_keys(vec![first, second.clone()], None),
        &tiers,
        true,
        &mut request,
    );
    assert!(armed);
    assert_eq!(request.prefetch.as_ref().unwrap().fetched, vec![second]);
}

fn arm_merges_cooldown_and_usage_keys() {
    let clock = TestClock::new();
    let cooldown = key("cooldown");
    let tpm = key("tpm");
    let rpm = key("rpm");
    let tiers = tiers(tier(&clock, None, 10.0), Some(tier(&clock, None, 10.0)));
    let mut request = RequestReads::default();
    let armed = RoutingPrefetch::arm(
        &read_keys(vec![cooldown.clone()], Some(usage(&["tpm"], &["rpm"]))),
        &tiers,
        true,
        &mut request,
    );
    assert!(armed);
    let prefetch = request.prefetch.as_ref().unwrap();
    assert_eq!(
        prefetch.fetched,
        vec![cooldown.clone(), tpm.clone(), rpm.clone()]
    );
    assert!(prefetch.keys.contains(&cooldown));
    assert!(prefetch.keys.contains(&tpm));
    assert!(prefetch.keys.contains(&rpm));
}

fn arm_with_nothing_due() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    seed(&cooldown, &second, counter(2));
    let tiers = tiers(cooldown, None);
    let mut request = RequestReads::default();
    let armed = RoutingPrefetch::arm(
        &read_keys(vec![first, second], None),
        &tiers,
        true,
        &mut request,
    );
    assert!(!armed);
    assert!(request.prefetch.is_none());
}

async fn read_uses_the_prefetch_once() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let tiers = tiers(cooldown, None);
    let store = MemoryStore::healthy(values(&[("b", 7)]));
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone(), second.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let cooldown_keys = [first.clone(), second.clone()];
    let results = read_once(&mut batch, &store, &tiers, &mut request, &cooldown_keys).await;
    assert_eq!(results.cooldown, vec![Some(counter(1)), Some(counter(7))]);
    assert_eq!(store.reads(), vec![vec![second.clone()]]);
    store.set(&second, counter(8));
    clock.set(601.0_f64);
    let again = read_once(&mut batch, &store, &tiers, &mut request, &cooldown_keys).await;
    assert_eq!(again.cooldown, vec![None, Some(counter(8))]);
    assert_eq!(
        store.reads(),
        vec![vec![second.clone()], vec![first, second]]
    );
}

async fn read_falls_back_after_a_failed_prefetch() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let tiers = tiers(cooldown, None);
    let store = MemoryStore::new(values(&[("b", 7)]), Some(ReadWarning::Unavailable));
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone(), second.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        &[first, second.clone()],
    )
    .await;
    assert_eq!(results.cooldown, vec![Some(counter(1)), None]);
    assert_eq!(results.warning, Some(ReadWarning::Unavailable));
    assert_eq!(store.reads(), vec![vec![second.clone()], vec![second]]);
}

async fn read_falls_back_when_the_prefetch_is_short() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, Some(10.0_f64), 10.0);
    seed(&cooldown, &second, counter(5));
    let tiers = tiers(cooldown, None);
    let store = MemoryStore::healthy(values(&[("a", 4), ("b", 9)]));
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone(), second.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    clock.set(11.0_f64);
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        &[first, second.clone()],
    )
    .await;
    assert_eq!(results.cooldown, vec![Some(counter(4)), Some(counter(9))]);
    assert_eq!(store.reads(), vec![vec![key("a")], vec![second]]);
}

async fn backfill_keeps_local_values() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let tiers = tiers(cooldown, None);
    let store = MemoryStore::healthy(values(&[("a", 9), ("b", 7)]));
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone(), second.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        &[first.clone(), second.clone()],
    )
    .await;
    assert_eq!(results.cooldown, vec![Some(counter(1)), Some(counter(7))]);
    assert_eq!(
        tiers.cooldown.batch_get(&[first, second.clone()]),
        vec![Some(counter(1)), Some(counter(7))]
    );
    assert_eq!(store.reads(), vec![vec![second]]);
}

async fn read_merges_the_usage_counters() {
    let clock = TestClock::new();
    let cooldown_key = key("deployment:a:cooldown");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &cooldown_key, counter(1));
    let tiers = tiers(cooldown, Some(tier(&clock, None, 10.0)));
    let store = MemoryStore::healthy(values(&[
        ("a:model:tpm:00-00", 5),
        ("a:model:rpm:00-00", 3),
    ]));
    let mut request = RequestReads::default();
    let usage_keys = usage(&["a:model:tpm:00-00"], &["a:model:rpm:00-00"]);
    let mut batch = RoutingReadBatch::for_strategy(Some(usage_keys.clone()), true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&cooldown_key),
    )
    .await;
    assert_usage_read(&batch, &tiers, &store, &results, &usage_keys);
}

async fn the_prefetched_usage_serves_the_next_read() {
    let clock = TestClock::new();
    let cooldown_key = key("deployment:a:cooldown");
    let tiers = tiers(tier(&clock, None, 10.0), Some(tier(&clock, None, 10.0)));
    seed(&tiers.cooldown, &cooldown_key, counter(1));
    let store = MemoryStore::healthy(values(&[
        ("a:model:tpm:00-00", 5),
        ("a:model:rpm:00-00", 3),
    ]));
    let mut request = RequestReads::default();
    let usage_keys = usage(&["a:model:tpm:00-00"], &["a:model:rpm:00-00"]);
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![cooldown_key.clone()], Some(usage_keys.clone())),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(Some(usage_keys.clone()), true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&cooldown_key),
    )
    .await;
    assert_usage_read(&batch, &tiers, &store, &results, &usage_keys);
}

fn assert_usage_read(
    batch: &RoutingReadBatch<RouteValue>,
    tiers: &ReadTiers<RouteValue>,
    store: &MemoryStore,
    results: &RoutingReadResults<RouteValue>,
    usage_keys: &UsageReadKeys,
) {
    let expected = Some(vec![Some(counter(5)), Some(counter(3))]);
    assert_eq!(results.cooldown, vec![Some(counter(1))]);
    assert_eq!(results.usage, expected);
    let prefetched = batch.prefetched_usage.as_ref().unwrap();
    assert_eq!(prefetched.values_for(&usage_keys.combined()), expected);
    assert_eq!(
        tiers
            .usage
            .as_ref()
            .unwrap()
            .batch_get(&usage_keys.combined()),
        vec![Some(counter(5)), Some(counter(3))]
    );
    assert_eq!(
        tiers.cooldown.batch_get(&usage_keys.combined()),
        vec![None, None]
    );
    assert_eq!(
        store.reads(),
        vec![vec![key("a:model:tpm:00-00"), key("a:model:rpm:00-00")]]
    );
}

async fn read_without_a_selector_keeps_no_prefetched_usage() {
    let clock = TestClock::new();
    let first = key("a");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let store = MemoryStore::healthy(values(&[("a", 3)]));
    let mut request = RequestReads::default();
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&first),
    )
    .await;
    assert_eq!(results.cooldown, vec![Some(counter(3))]);
    assert!(results.usage.is_none());
    assert!(batch.prefetched_usage.is_none());
}

async fn requests_are_isolated() {
    let clock = TestClock::new();
    let first = key("a");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let store = MemoryStore::healthy(values(&[("a", 3)]));
    let mut first_request = RequestReads::default();
    let mut second_request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone()], None),
        &tiers,
        true,
        &mut first_request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    // the first request's arm already reserved the shared read throttle for "a"
    let second_results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut second_request,
        slice::from_ref(&first),
    )
    .await;
    assert_eq!(second_results.cooldown, vec![None]);
    assert!(first_request.prefetch.is_some());
    let first_results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut first_request,
        slice::from_ref(&first),
    )
    .await;
    assert_eq!(first_results.cooldown, vec![Some(counter(3))]);
    assert!(first_request.prefetch.is_none());
    assert_eq!(store.reads(), vec![vec![first]]);
}

fn cancelled_read_releases_the_reservation() {
    let clock = TestClock::new();
    let first = key("a");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let store = HangingStore::new();
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let needed = [first.clone()];
    let pending = {
        let mut session = RoutingReadSession {
            store: &store,
            tiers: &tiers,
            request: &mut request,
        };
        let mut read = Box::pin(batch.read_routing_state(&mut session, &needed));
        let mut context = Context::from_waker(Waker::noop());
        read.as_mut().poll(&mut context).is_pending()
    };
    assert!(pending);
    let prepared = tiers.cooldown.prepare(slice::from_ref(&first));
    assert_eq!(prepared.reservation.due.len(), 1);
}

fn prefetched_usage_covers_the_strategy_keys() {
    let tpm = key("tpm");
    let rpm = key("rpm");
    let all = vec![tpm.clone(), rpm.clone()];
    let prefetched = PrefetchedUsage {
        keys: all.clone(),
        values: Some(BTreeMap::from([
            (tpm, Some(counter(5))),
            (rpm, Some(counter(3))),
        ])),
    };
    assert!(prefetched.covers(&all));
    assert!(!prefetched.covers(&[key("other")]));
    assert_eq!(
        prefetched.values_for(&all),
        Some(vec![Some(counter(5)), Some(counter(3))])
    );
    assert_eq!(prefetched.values_for(&[key("other")]), Some(vec![None]));
}

fn prefetched_usage_values_are_none_when_failed() {
    let all = keys(&["tpm"]);
    let prefetched = PrefetchedUsage::<RouteValue> {
        keys: all.clone(),
        values: None,
    };
    assert!(prefetched.covers(&all));
    assert_eq!(prefetched.values_for(&all), None);
}

fn for_strategy_uses_usage_keys_without_a_prefetch() {
    let batch =
        RoutingReadBatch::<RouteValue>::for_strategy(Some(usage(&["tpm"], &[])), false).unwrap();
    assert_eq!(batch.usage.unwrap().tpm, keys(&["tpm"]));
}

fn for_strategy_needs_an_armed_prefetch_without_usage() {
    let batch = RoutingReadBatch::<RouteValue>::for_strategy(None, true).unwrap();
    assert!(batch.usage.is_none());
    assert!(RoutingReadBatch::<RouteValue>::for_strategy(None, false).is_none());
}

fn for_strategy_keeps_no_usage_keys() {
    let batch = RoutingReadBatch::<RouteValue>::for_strategy(None, true).unwrap();
    assert!(batch.usage.is_none());
    assert!(batch.prefetched_usage.is_none());
}
