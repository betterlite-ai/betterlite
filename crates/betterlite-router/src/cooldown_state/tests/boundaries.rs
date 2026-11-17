use std::future::Future as _;
use std::sync::Arc;
use std::task::{Context, Waker};

use super::support::{
    MemoryCooldowns, MemoryHealth, TestClock, active_at, cache, health, ids, record, settings,
    snapshot, unhealthy, unhealthy_ids,
};
use crate::cooldown_state::cooldown::{ActiveCooldown, CooldownCache};
use crate::cooldown_state::health::extract_unhealthy;
use crate::routing_read_batch::{RequestReads, RoutingReadBatch};
use crate::routing_value_store::{MAX_KEY_BYTES, StoreError};

pub(crate) async fn run(name: &str) {
    match name {
        "a_cooldown_ending_exactly_now_is_gone" => a_cooldown_ending_now_is_gone().await,
        "a_cooldown_ending_just_after_now_stays" => a_cooldown_ending_after_now_stays().await,
        "the_correction_starts_past_the_five_second_margin" => the_five_second_margin().await,
        "the_corrected_ttl_is_capped_at_a_minute" => the_cap_is_a_minute().await,
        "the_corrected_ttl_uses_the_shorter_remaining_time" => the_shorter_remaining().await,
        "an_unhealthy_state_exactly_at_the_staleness_threshold_is_stale" => the_staleness_edge(),
        "a_state_exactly_at_the_merge_expiry_is_pruned" => the_merge_expiry().await,
        "the_staleness_ttl_truncates_a_fractional_expiry" => the_fractional_expiry().await,
        "a_key_at_the_length_limit_is_accepted_and_one_past_it_is_not" => the_key_limit().await,
        "a_cancelled_read_leaves_the_key_readable" => a_cancelled_read().await,
        "a_re_add_at_the_exact_expiry_takes_the_new_ttl" => a_re_add_at_the_expiry().await,
        "set_states_replaces_a_local_ttl_ending_exactly_now" => set_states_at_the_expiry().await,
        "the_minimum_cooldown_of_a_zero_entry_is_zero" => a_zero_cooldown_stays_zero(),
        _ => panic!("unknown case {name}"),
    }
}

fn poll_once(cache: &CooldownCache, model_ids: &[String]) -> bool {
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let mut read = Box::pin(cache.read(&mut batch, &mut request, model_ids));
    let mut context = Context::from_waker(Waker::noop());
    read.as_mut().poll(&mut context).is_pending()
}

async fn a_cooldown_ending_now_is_gone() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 5.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert!(active_at(&cache, &clock, 5.0, "a").await.is_empty());
}

async fn a_cooldown_ending_after_now_stays() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 5.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert_eq!(
        active_at(&cache, &clock, 4.75, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 0.25,
            record: record(0.0, 5.0),
        }]
    );
}

async fn the_five_second_margin() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 600.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    // the memory expiry equals now + remaining + 5 exactly, so no correction happens and the promoted copy stays
    let margin = active_at(&cache, &clock, 5.0, "a").await;
    assert_eq!(margin[0].remaining_seconds.to_bits(), 595.0_f64.to_bits());
    shared.set("a", record(65.0, 30.0));
    let stale = active_at(&cache, &clock, 70.0, "a").await;
    assert_eq!(stale[0].record.timestamp.to_bits(), 0.0_f64.to_bits());
    assert_eq!(stale[0].remaining_seconds.to_bits(), 530.0_f64.to_bits());
}

async fn the_cap_is_a_minute() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 120.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert_eq!(active_at(&cache, &clock, 1.0, "a").await.len(), 1);
    // the shared entry is now a longer cooldown: only a corrected memory copy can still answer with the first one
    shared.set("a", record(100.0, 600.0));
    let corrected = active_at(&cache, &clock, 60.5, "a").await;
    assert_eq!(
        corrected[0].record.cooldown_time.to_bits(),
        120.0_f64.to_bits()
    );
    let reread = active_at(&cache, &clock, 61.5, "a").await;
    assert_eq!(
        reread[0].record.cooldown_time.to_bits(),
        600.0_f64.to_bits()
    );
    assert_eq!(reread[0].remaining_seconds.to_bits(), 638.5_f64.to_bits());
}

async fn the_shorter_remaining() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 30.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert_eq!(
        active_at(&cache, &clock, 1.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 29.0,
            record: record(0.0, 30.0),
        }]
    );
    // the corrected expiry of 29 seconds has passed, so the newer shared entry answers instead of the old copy
    shared.set("a", record(25.0, 30.0));
    let promoted = active_at(&cache, &clock, 31.0, "a").await;
    assert_eq!(promoted[0].record.timestamp.to_bits(), 25.0_f64.to_bits());
    assert_eq!(promoted[0].remaining_seconds.to_bits(), 24.0_f64.to_bits());
}

fn the_staleness_edge() {
    let stored = snapshot(&[("a", unhealthy(0.0))]);
    assert!(extract_unhealthy(Some(&stored), 10.0, 10.0).is_empty());
    assert_eq!(
        extract_unhealthy(Some(&stored), 9.75, 10.0),
        unhealthy_ids(&["a"])
    );
    // a verdict taken five seconds ago is still fresh at the same instant, since the threshold counts back from the moment the probe ran
    let recent = snapshot(&[("b", unhealthy(5.0))]);
    assert_eq!(
        extract_unhealthy(Some(&recent), 10.0, 10.0),
        unhealthy_ids(&["b"])
    );
}

async fn the_merge_expiry() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    shared.set_raw(snapshot(&[("a", unhealthy(0.0))]));
    clock.set(15.0);
    cache.set_states(&snapshot(&[])).unwrap();
    assert!(shared.writes().last().unwrap().0.is_empty());
    shared.set_raw(snapshot(&[("a", unhealthy(0.25))]));
    cache.set_states(&snapshot(&[])).unwrap();
    assert_eq!(
        shared.writes().last().unwrap().0,
        snapshot(&[("a", unhealthy(0.25))])
    );
}

async fn the_fractional_expiry() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 2.5);
    cache
        .set_states(&snapshot(&[("a", unhealthy(0.0))]))
        .unwrap();
    assert_eq!(
        shared.writes(),
        vec![(snapshot(&[("a", unhealthy(0.0))]), 3)]
    );
}

async fn the_key_limit() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    let at_limit = "a".repeat(MAX_KEY_BYTES.saturating_sub(20));
    cache.add(&at_limit, "boom", 429, None).unwrap();
    assert_eq!(shared.writes().len(), 1);
    let past = "a".repeat(MAX_KEY_BYTES.saturating_sub(19));
    let bytes = format!("deployment:{past}:cooldown").len();
    let add_error = cache.add(&past, "boom", 429, None).unwrap_err();
    assert!(
        matches!(add_error, StoreError::KeyTooLong { bytes: seen, limit } if seen == bytes && limit == MAX_KEY_BYTES)
    );
    let keys = ids(&[past.as_str()]);
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let read_error = cache
        .read(&mut batch, &mut request, &keys)
        .await
        .unwrap_err();
    assert!(matches!(read_error, StoreError::KeyTooLong { .. }));
    let active_error = cache
        .active_from_results(&keys, &[Some(record(0.0, 5.0))])
        .unwrap_err();
    assert!(matches!(active_error, StoreError::KeyTooLong { .. }));
    assert_eq!(shared.writes().len(), 1);
}

async fn a_cancelled_read() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default().hanging());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    let keys = ids(&["a"]);
    clock.set(1.0);
    assert!(poll_once(&cache, &keys));
    shared.stop_hanging();
    shared.set("a", record(0.0, 5.0));
    assert_eq!(
        active_at(&cache, &clock, 2.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 3.0,
            record: record(0.0, 5.0),
        }]
    );
}

async fn a_re_add_at_the_expiry() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    cache.add("a", "boom", 429, Some(5.0)).unwrap();
    clock.set(5.0);
    cache.add("a", "boom", 429, Some(30.0)).unwrap();
    // InMemoryCache.set_cache prunes a TTL ending exactly now, so the re-add replaces it instead of keeping it
    assert_eq!(
        cache.local_expiries(&ids(&["a"])).unwrap(),
        vec![Some(35.0)]
    );
}

async fn set_states_at_the_expiry() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    cache
        .set_states(&snapshot(&[("a", unhealthy(0.0))]))
        .unwrap();
    clock.set(15.0);
    cache
        .set_states(&snapshot(&[("b", unhealthy(15.0))]))
        .unwrap();
    assert_eq!(cache.local_expiry(), Some(30.0));
}

fn a_zero_cooldown_stays_zero() {
    // an explicit 0 is used as given: LiteLLM's "min or default" is Python truthiness, which docs/compatibility.md drops
    let clock = TestClock::new();
    let cache = cache(&clock, Arc::new(MemoryCooldowns::default()), &settings());
    let results = [Some(record(0.0, 0.0))];
    assert_eq!(
        cache.minimum_cooldown(&results).to_bits(),
        0.0_f64.to_bits()
    );
}
