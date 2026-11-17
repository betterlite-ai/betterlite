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
    assert_eq!(
        active_at(&cache, &clock, 5.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 595.0,
            record: record(0.0, 600.0),
        }]
    );
    shared.set("a", record(65.0, 30.0));
    assert_eq!(
        active_at(&cache, &clock, 70.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 530.0,
            record: record(0.0, 600.0),
        }]
    );
}

async fn the_cap_is_a_minute() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 120.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert_eq!(
        active_at(&cache, &clock, 1.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 119.0,
            record: record(0.0, 120.0),
        }]
    );
    shared.set("a", record(60.0, 30.0));
    assert_eq!(
        active_at(&cache, &clock, 62.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 28.0,
            record: record(60.0, 30.0),
        }]
    );
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
    shared.set("a", record(25.0, 30.0));
    assert_eq!(
        active_at(&cache, &clock, 31.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 24.0,
            record: record(25.0, 30.0),
        }]
    );
}

fn the_staleness_edge() {
    let stored = snapshot(&[("a", unhealthy(0.0))]);
    assert!(extract_unhealthy(Some(&stored), 10.0, 10.0).is_empty());
    assert_eq!(
        extract_unhealthy(Some(&stored), 9.75, 10.0),
        unhealthy_ids(&["a"])
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
    let error = cache.add(&past, "boom", 429, None).unwrap_err();
    assert!(
        matches!(error, StoreError::KeyTooLong { bytes: seen, limit } if seen == bytes && limit == MAX_KEY_BYTES)
    );
    let keys = ids(&[past.as_str()]);
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let error = cache
        .read(&mut batch, &mut request, &keys)
        .await
        .unwrap_err();
    assert!(matches!(error, StoreError::KeyTooLong { .. }));
    let error = cache
        .active_from_results(&keys, &[Some(record(0.0, 5.0))])
        .unwrap_err();
    assert!(matches!(error, StoreError::KeyTooLong { .. }));
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
