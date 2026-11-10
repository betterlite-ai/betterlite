use std::slice;

use super::support::{
    MemoryStore, TestClock, counter, key, read_keys, read_once, seed, tier, tiers, usage, values,
};
use crate::routing_read_batch::local::{DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_READ_EXPIRY_SECONDS};
use crate::routing_read_batch::store::ReadWarning;
use crate::routing_read_batch::{RequestReads, RoutingPrefetch, RoutingReadBatch};

pub(crate) async fn run(name: &str) {
    match name {
        "reservation_is_due_at_the_exact_expiry" => reservation_is_due_at_the_exact_expiry(),
        "reservation_age_is_the_elapsed_time" => reservation_age_is_the_elapsed_time(),
        "default_read_expiry_is_ten_seconds" => default_read_expiry_is_ten_seconds(),
        "take_requires_every_needed_key" => take_requires_every_needed_key(),
        "take_releases_a_short_prefetch" => take_releases_a_short_prefetch(),
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
