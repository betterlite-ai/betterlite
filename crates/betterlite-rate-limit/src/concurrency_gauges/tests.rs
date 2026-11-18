#![cfg(test)]

pub(crate) mod support;

use std::sync::Arc;

use crate::rate_limit_window_store::CounterScript;

use self::support::{FixedClock, PlannedScript, RecordingCache, gauge, gauges_with, int_values};
use super::gauge_times;

const NOW: f64 = 1_700_000_000.0;
const ONE_TTL_EARLIER: f64 = 1_699_996_400.0;

#[test]
fn cutoff_reaches_back_one_slot_ttl_from_the_local_clock() {
    let clock = FixedClock::at(NOW);
    let times = gauge_times(clock.as_ref()).expect("a clock after the Unix epoch");
    assert_eq!(times.now.to_bits(), NOW.to_bits());
    assert_eq!(times.cutoff.to_bits(), ONE_TTL_EARLIER.to_bits());
}

#[test]
fn cutoff_is_unbounded_when_the_clock_is_before_one_slot_ttl() {
    let clock = FixedClock::at(100.0);
    let times = gauge_times(clock.as_ref()).expect("a clock after the Unix epoch");
    assert_eq!(times.now.to_bits(), 100.0_f64.to_bits());
    assert!(times.cutoff.is_infinite());
    assert!(times.cutoff.is_sign_negative());
}

#[tokio::test]
async fn read_only_checks_use_the_count_script_and_never_the_acquire_one() {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(NOW);
    let count = PlannedScript::new("count", vec![int_values(&[1])]);
    let acquire = PlannedScript::new("acquire", vec![int_values(&[0, 1])]);
    let mut gauges = gauges_with(Arc::clone(&cache), clock);
    let count_script: Arc<dyn CounterScript> = Arc::clone(&count);
    let acquire_script: Arc<dyn CounterScript> = Arc::clone(&acquire);
    gauges.count_script = Some(count_script);
    gauges.acquire_script = Some(acquire_script);
    let response = gauges
        .check_parallel_request_gauges(&[gauge("g1", 5)], "slot-a", true)
        .await
        .expect("a read-only gauge response");
    assert_eq!(response.overall_code, "OK");
    assert_eq!(count.calls().len(), 1);
    assert!(acquire.calls().is_empty());
    assert!(cache.batch_reads().is_empty());
}

#[tokio::test]
async fn acquire_checks_use_the_acquire_script_and_never_the_count_one() {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(NOW);
    let count = PlannedScript::new("count", vec![int_values(&[9])]);
    let acquire = PlannedScript::new("acquire", vec![int_values(&[0, 1])]);
    let mut gauges = gauges_with(Arc::clone(&cache), clock);
    let count_script: Arc<dyn CounterScript> = Arc::clone(&count);
    let acquire_script: Arc<dyn CounterScript> = Arc::clone(&acquire);
    gauges.count_script = Some(count_script);
    gauges.acquire_script = Some(acquire_script);
    let response = gauges
        .check_parallel_request_gauges(&[gauge("g1", 5)], "slot-a", false)
        .await
        .expect("an acquiring gauge response");
    assert_eq!(response.overall_code, "OK");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 4);
    assert_eq!(acquire.calls().len(), 1);
    assert!(count.calls().is_empty());
}
