use std::collections::BTreeMap;
use std::slice;

use super::support::{
    BreakerTestClock, MemoryStore, TestClock, counter, key, read_keys, read_once, real_store, seed,
    seed_remote, tier, tier_capped, tiers, usage, values,
};
use crate::routing_read_batch::local::{DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_READ_EXPIRY_SECONDS};
use crate::routing_read_batch::read::apply_prefetched;
use crate::routing_read_batch::store::ReadWarning;
use crate::routing_read_batch::{RequestReads, RoutingPrefetch, RoutingReadBatch};
use crate::routing_state_store::test_support::FakeRedis;

pub(crate) async fn run(name: &str) {
    match name {
        "reservation_is_due_at_the_exact_expiry" => reservation_is_due_at_the_exact_expiry(),
        "reservation_age_is_the_elapsed_time" => reservation_age_is_the_elapsed_time(),
        "default_read_expiry_is_ten_seconds" => default_read_expiry_is_ten_seconds(),
        "take_requires_every_needed_key" => take_requires_every_needed_key(),
        "take_releases_a_short_prefetch" => take_releases_a_short_prefetch(),
        "local_entries_are_capped" => local_entries_are_capped(),
        "last_reads_are_capped" => last_reads_are_capped(),
        "apply_prefetched_requires_a_covered_key" => apply_prefetched_requires_a_covered_key(),
        "local_value_is_readable_at_its_exact_expiry" => {
            local_value_is_readable_at_its_exact_expiry();
        }
        "default_local_ttl_is_six_hundred_seconds" => {
            default_local_ttl_is_six_hundred_seconds();
        }
        "backfill_ttl_comes_from_the_tier" => backfill_ttl_comes_from_the_tier(),
        "empty_keys_never_arm_or_read" => empty_keys_never_arm_or_read().await,
        "usage_without_keys_still_tracks_the_selector" => {
            usage_without_keys_still_tracks_the_selector().await;
        }
        "circuit_open_keeps_the_local_values" => circuit_open_keeps_the_local_values().await,
        "unavailable_keeps_the_reservation" => unavailable_keeps_the_reservation().await,
        "an_absent_remote_value_still_consumes_the_prefetch" => {
            an_absent_remote_value_still_consumes_the_prefetch().await;
        }
        "duplicate_cooldown_keys_are_read_once" => duplicate_cooldown_keys_are_read_once().await,
        "real_store_opens_the_circuit_and_releases" => {
            real_store_opens_the_circuit_and_releases().await;
        }
        "real_store_keeps_the_reservation_when_unavailable" => {
            real_store_keeps_the_reservation_when_unavailable().await;
        }
        "real_store_serves_a_recovered_probe" => real_store_serves_a_recovered_probe().await,
        _ => panic!("unknown case {name}"),
    }
}

fn reservation_is_due_at_the_exact_expiry() {
    let clock = TestClock::new();
    let first = key("a");
    let cooldown = tier(&clock, None, 5.0);
    assert_eq!(
        cooldown.prepare(slice::from_ref(&first)).reservation.due,
        vec![first.clone()]
    );
    clock.set(4.999_f64);
    assert!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .is_empty()
    );
    clock.set(5.0_f64);
    assert_eq!(
        cooldown.prepare(slice::from_ref(&first)).reservation.due,
        vec![first]
    );
}

fn reservation_age_is_the_elapsed_time() {
    let clock = TestClock::new();
    let first = key("a");
    let cooldown = tier(&clock, None, 10.0);
    cooldown
        .state
        .lock()
        .unwrap()
        .last_reads
        .insert(first.clone(), 100.0_f64);
    clock.set(105.0_f64);
    assert!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .is_empty()
    );
    clock.set(110.0_f64);
    assert_eq!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .len(),
        1
    );
}

fn default_read_expiry_is_ten_seconds() {
    assert_eq!(DEFAULT_READ_EXPIRY_SECONDS.to_bits(), 10.0_f64.to_bits());
    let clock = TestClock::new();
    let first = key("a");
    let cooldown = tier(&clock, None, DEFAULT_READ_EXPIRY_SECONDS);
    assert_eq!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .len(),
        1
    );
    clock.set(9.999_f64);
    assert!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .is_empty()
    );
    clock.set(10.0_f64);
    assert_eq!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .len(),
        1
    );
}

fn take_requires_every_needed_key() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    assert!(request.take(slice::from_ref(&first)).is_some());
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    assert!(request.take(&[first.clone(), second]).is_none());
    assert_eq!(
        tiers
            .cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .len(),
        1
    );
}

fn take_releases_a_short_prefetch() {
    let clock = TestClock::new();
    let first = key("a");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![first.clone()], None),
        &tiers,
        true,
        &mut request,
    ));
    assert!(request.take(&[first.clone(), key("b")]).is_none());
    assert_eq!(
        tiers
            .cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .len(),
        1
    );
}

fn local_value_is_readable_at_its_exact_expiry() {
    let clock = TestClock::new();
    let first = key("a");
    let cooldown = tier(&clock, Some(10.0_f64), 10.0);
    seed(&cooldown, &first, counter(1));
    clock.set(10.0_f64);
    assert_eq!(
        cooldown.batch_get(slice::from_ref(&first)),
        vec![Some(counter(1))]
    );
    clock.set(10.001_f64);
    assert_eq!(cooldown.batch_get(slice::from_ref(&first)), vec![None]);
}

fn default_local_ttl_is_six_hundred_seconds() {
    assert_eq!(DEFAULT_LOCAL_TTL_SECONDS.to_bits(), 600.0_f64.to_bits());
    let clock = TestClock::new();
    let first = key("a");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    clock.set(600.0_f64);
    assert_eq!(
        cooldown.batch_get(slice::from_ref(&first)),
        vec![Some(counter(1))]
    );
    clock.set(600.001_f64);
    assert_eq!(cooldown.batch_get(slice::from_ref(&first)), vec![None]);
}

fn backfill_ttl_comes_from_the_tier() {
    let clock = TestClock::new();
    let first = key("a");
    let cooldown = tier(&clock, Some(5.0_f64), 10.0);
    seed(&cooldown, &first, counter(1));
    clock.set(5.0_f64);
    assert_eq!(
        cooldown.batch_get(slice::from_ref(&first)),
        vec![Some(counter(1))]
    );
    clock.set(5.001_f64);
    assert_eq!(cooldown.batch_get(slice::from_ref(&first)), vec![None]);
}

async fn empty_keys_never_arm_or_read() {
    let clock = TestClock::new();
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let store = MemoryStore::healthy(values(&[("a", 1)]));
    let mut request = RequestReads::default();
    assert!(!RoutingPrefetch::arm(
        &read_keys(Vec::new(), None),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let results = read_once(&mut batch, &store, &tiers, &mut request, &[]).await;
    assert!(results.cooldown.is_empty());
    assert!(results.usage.is_none());
    assert!(store.reads().is_empty());
}

async fn usage_without_keys_still_tracks_the_selector() {
    let clock = TestClock::new();
    let cooldown = key("cooldown");
    let tiers = tiers(tier(&clock, None, 10.0), Some(tier(&clock, None, 10.0)));
    let store = MemoryStore::healthy(values(&[("cooldown", 2)]));
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &read_keys(vec![cooldown.clone()], Some(usage(&[], &[]))),
        &tiers,
        true,
        &mut request,
    ));
    let mut batch = RoutingReadBatch::for_strategy(Some(usage(&[], &[])), true).unwrap();
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&cooldown),
    )
    .await;
    assert_eq!(results.usage, Some(Vec::new()));
    let prefetched = batch.prefetched_usage.as_ref().unwrap();
    assert!(prefetched.keys.is_empty());
    assert_eq!(prefetched.values_for(&[]), Some(Vec::new()));
}

async fn circuit_open_keeps_the_local_values() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let tiers = tiers(cooldown, None);
    let store = MemoryStore::new(values(&[("b", 2)]), Some(ReadWarning::CircuitOpen));
    let mut request = RequestReads::default();
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
    assert_eq!(results.warning, Some(ReadWarning::CircuitOpen));
    assert_eq!(
        tiers
            .cooldown
            .prepare(slice::from_ref(&second))
            .reservation
            .due
            .len(),
        1
    );
}

async fn unavailable_keeps_the_reservation() {
    let clock = TestClock::new();
    let first = key("b");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let store = MemoryStore::new(values(&[("b", 2)]), Some(ReadWarning::Unavailable));
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
    assert_eq!(results.warning, Some(ReadWarning::Unavailable));
    assert!(
        tiers
            .cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .is_empty()
    );
}

async fn an_absent_remote_value_still_consumes_the_prefetch() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let tiers = tiers(cooldown, None);
    let store = MemoryStore::healthy(values(&[]));
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
    assert_eq!(store.reads(), vec![vec![second]]);
    assert!(request.prefetch.is_none());
}

async fn duplicate_cooldown_keys_are_read_once() {
    let clock = TestClock::new();
    let first = key("a");
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let store = MemoryStore::healthy(values(&[("a", 2)]));
    let mut request = RequestReads::default();
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let cooldown_keys = [first.clone(), first.clone()];
    let results = read_once(&mut batch, &store, &tiers, &mut request, &cooldown_keys).await;
    assert_eq!(results.cooldown, vec![Some(counter(2)), Some(counter(2))]);
    assert_eq!(store.reads(), vec![vec![first]]);
}

fn local_entries_are_capped() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let third = key("c");
    let cooldown = tier_capped(&clock, 2, 1_000);
    clock.set(0.0_f64);
    seed(&cooldown, &first, counter(0));
    clock.set(1.0_f64);
    seed(&cooldown, &second, counter(1));
    clock.set(2.0_f64);
    seed(&cooldown, &third, counter(2));
    assert_eq!(
        cooldown.batch_get(&[first, second, third]),
        vec![None, Some(counter(1)), Some(counter(2))]
    );
}

fn last_reads_are_capped() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let third = key("c");
    let cooldown = tier_capped(&clock, 200, 2);
    clock.set(0.0_f64);
    assert_eq!(
        cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .len(),
        1
    );
    clock.set(1.0_f64);
    assert_eq!(
        cooldown
            .prepare(slice::from_ref(&second))
            .reservation
            .due
            .len(),
        1
    );
    clock.set(2.0_f64);
    assert_eq!(
        cooldown
            .prepare(slice::from_ref(&third))
            .reservation
            .due
            .len(),
        1
    );
    let state = cooldown.state.lock().unwrap();
    assert!(!state.last_reads.contains_key(&first));
    assert!(state.last_reads.contains_key(&second));
    assert!(state.last_reads.contains_key(&third));
    drop(state);
}

async fn real_store_opens_the_circuit_and_releases() {
    let server = FakeRedis::try_start().await.unwrap();
    let breaker_clock = BreakerTestClock::new();
    let store = real_store(&server, 1, 60_000, &breaker_clock);
    server.set_down(true);
    let clock = TestClock::new();
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let warm_key = key("deployment:a:cooldown");
    let warm = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&warm_key),
    )
    .await;
    assert_eq!(warm.warning, Some(ReadWarning::Unavailable));
    let open_key = key("deployment:b:cooldown");
    let opened = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&open_key),
    )
    .await;
    assert_eq!(opened.warning, Some(ReadWarning::CircuitOpen));
    assert_eq!(
        tiers
            .cooldown
            .prepare(slice::from_ref(&open_key))
            .reservation
            .due
            .len(),
        1
    );
}

async fn real_store_keeps_the_reservation_when_unavailable() {
    let server = FakeRedis::try_start().await.unwrap();
    let breaker_clock = BreakerTestClock::new();
    let store = real_store(&server, 100, 60_000, &breaker_clock);
    server.set_down(true);
    let clock = TestClock::new();
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let first = key("deployment:a:cooldown");
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&first),
    )
    .await;
    assert_eq!(results.warning, Some(ReadWarning::Unavailable));
    assert!(
        tiers
            .cooldown
            .prepare(slice::from_ref(&first))
            .reservation
            .due
            .is_empty()
    );
}

async fn real_store_serves_a_recovered_probe() {
    let server = FakeRedis::try_start().await.unwrap();
    let breaker_clock = BreakerTestClock::new();
    let store = real_store(&server, 1, 100, &breaker_clock);
    server.set_down(true);
    let clock = TestClock::new();
    let tiers = tiers(tier(&clock, None, 10.0), None);
    let mut request = RequestReads::default();
    let mut batch = RoutingReadBatch::for_strategy(None, true).unwrap();
    let warm_key = key("deployment:a:cooldown");
    let warm = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&warm_key),
    )
    .await;
    assert_eq!(warm.warning, Some(ReadWarning::Unavailable));
    breaker_clock.set_millis(200);
    server.set_down(false);
    let recovered = key("deployment:b:cooldown");
    seed_remote(&server, &recovered, &counter(7));
    let results = read_once(
        &mut batch,
        &store,
        &tiers,
        &mut request,
        slice::from_ref(&recovered),
    )
    .await;
    assert_eq!(results.cooldown, vec![Some(counter(7))]);
    assert!(results.warning.is_none());
    assert_eq!(
        tiers.cooldown.batch_get(slice::from_ref(&recovered)),
        vec![Some(counter(7))]
    );
}

fn apply_prefetched_requires_a_covered_key() {
    let clock = TestClock::new();
    let first = key("a");
    let second = key("b");
    let cooldown = tier(&clock, None, 10.0);
    seed(&cooldown, &first, counter(1));
    let fetched = vec![second.clone()];
    let values = BTreeMap::from([(second.clone(), counter(7))]);
    assert_eq!(
        apply_prefetched(&cooldown, &[first.clone(), second], &fetched, &values),
        Some(vec![Some(counter(1)), Some(counter(7))])
    );
    assert_eq!(
        apply_prefetched(&cooldown, &[first, key("c")], &fetched, &values),
        None
    );
}
