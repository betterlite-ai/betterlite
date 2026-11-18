#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, PlannedScript, RecordingCache, ReplyPlan, gauge, gauges_with,
    int_counter, int_values, key_list, registry,
};
use crate::concurrency_gauges::{ConcurrencyGauges, GaugeError, PARALLEL_SLOT_TTL};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::CacheCounterValue;
use crate::rate_limit_window_store::CounterScript;

use super::check_acquire;

const NOW: f64 = 1_700_000_000.0;

fn acquire_gauges(
    cache: Arc<RecordingCache>,
    plans: Vec<ReplyPlan>,
) -> (ConcurrencyGauges, Arc<PlannedScript>) {
    let mut gauges = gauges_with(cache, FixedClock::at(NOW));
    let handle = PlannedScript::new("acquire", plans);
    let script: Arc<dyn CounterScript> = Arc::clone(&handle);
    gauges.acquire_script = Some(script);
    (gauges, handle)
}

fn mirror_write(key: &str, count: i64) -> CacheWrite {
    CacheWrite {
        key: key.to_owned(),
        value: int_counter(count),
        ttl: PARALLEL_SLOT_TTL,
        local_only: true,
    }
}

#[tokio::test]
async fn precheck_rejects_without_calling_the_acquire_script() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    let (gauges, script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[0, 2])]);
    let response = check_acquire(&gauges, &[gauge("g1", 2)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an over-limit pre-check");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one rejecting status");
    assert_eq!(status.code, "OVER_LIMIT");
    assert_eq!(status.current_limit, 2);
    assert_eq!(status.limit_remaining, 0);
    assert!(script.calls().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn precheck_lets_a_count_below_the_limit_reach_the_script() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(1));
    let (gauges, script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[2])]);
    let response = check_acquire(&gauges, &[gauge("g1", 2)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an acquired gauge");
    assert_eq!(response.overall_code, "OK");
    assert_eq!(script.calls().len(), 1);
    assert_eq!(cache.writes(), vec![mirror_write("g1", 2)]);
}

#[tokio::test]
async fn redis_success_mirrors_counts_and_returns_statuses() {
    let cache = RecordingCache::new();
    let (gauges, script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[1, 2])]);
    let response = check_acquire(
        &gauges,
        &[gauge("g1", 5), gauge("g2", 4)],
        &key_list(&["g1", "g2"]),
        "slot-a",
    )
    .await
    .expect("an acquired gauge");
    assert_eq!(response.overall_code, "OK");
    let remaining: Vec<i64> = response
        .statuses
        .iter()
        .map(|status| status.limit_remaining)
        .collect();
    assert_eq!(remaining, vec![4, 2]);
    assert_eq!(
        cache.writes(),
        vec![mirror_write("g1", 1), mirror_write("g2", 2)]
    );
    assert_eq!(
        script.calls(),
        vec![(
            key_list(&["g1", "g2"]),
            vec![
                RedisScalar::Text("5".to_owned()),
                RedisScalar::Text("3600".to_owned()),
                RedisScalar::Text("slot-a".to_owned()),
                RedisScalar::Text("4".to_owned()),
                RedisScalar::Text("3600".to_owned()),
                RedisScalar::Text("slot-a".to_owned()),
            ],
        )]
    );
}

#[tokio::test]
async fn redis_success_over_the_limit_names_the_rejecting_gauge() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[1, 2, 3, 4])]);
    let response = check_acquire(
        &gauges,
        &[gauge("g1", 5), gauge("g2", 4)],
        &key_list(&["g1", "g2"]),
        "slot-a",
    )
    .await
    .expect("an over-limit acquisition");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one rejecting status");
    assert_eq!(status.code, "OVER_LIMIT");
    assert_eq!(status.current_limit, 4);
    assert_eq!(status.limit_remaining, 1);
    assert_eq!(status.descriptor_key, "api-key");
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn redis_success_mirrors_text_counts() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Values(vec![
            CacheCounterValue::Int(0),
            CacheCounterValue::Text("4".to_owned()),
        ])],
    );
    let response = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an acquired gauge");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 1);
}

#[tokio::test]
async fn redis_success_mirrors_a_negative_count_as_given() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[0, -1])]);
    let response = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an acquired gauge");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 6);
    assert_eq!(cache.writes(), vec![mirror_write("g1", -1)]);
}

#[tokio::test]
async fn counts_beyond_the_gauge_list_are_ignored() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[0, 1, 2, 3])]);
    let response = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an acquired gauge");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(cache.writes(), vec![mirror_write("g1", 1)]);
}

#[tokio::test]
async fn redis_failure_falls_back_to_in_memory() {
    let cache = RecordingCache::new();
    let (gauges, script) = acquire_gauges(Arc::clone(&cache), vec![ReplyPlan::Failure]);
    let response = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an in-memory gauge acquisition");
    assert_eq!(response.overall_code, "OK");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 4);
    assert_eq!(script.calls().len(), 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-a", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn redis_failure_rejects_when_fail_closed() {
    let cache = RecordingCache::new();
    let (mut gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![ReplyPlan::Failure]);
    gauges.fail_closed = FailClosedPolicy { enabled: true };
    let error = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect_err("a fail-closed rejection");
    assert!(matches!(
        &error,
        GaugeError::Unverifiable(rejection) if rejection.status_code == 503
    ));
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_fallback_when_script_is_not_registered() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-old", NOW)]));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = check_acquire(&gauges, &[gauge("g1", 3)], &key_list(&["g1"]), "slot-a")
        .await
        .expect("an in-memory gauge acquisition");
    assert_eq!(response.overall_code, "OK");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-a", NOW), ("slot-old", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_zero_gauge_index_is_an_error() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[1, 0, 2, 3])]);
    let error = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect_err("a gauge index of zero");
    assert!(matches!(
        error,
        GaugeError::GaugeIndex {
            index: 0,
            gauges: 1
        }
    ));
}

#[tokio::test]
async fn a_gauge_index_past_the_gauges_is_an_error() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[1, 3, 2, 3])]);
    let error = check_acquire(
        &gauges,
        &[gauge("g1", 5), gauge("g2", 5)],
        &key_list(&["g1", "g2"]),
        "slot-a",
    )
    .await
    .expect_err("a gauge index past the gauges");
    assert!(matches!(
        error,
        GaugeError::GaugeIndex {
            index: 3,
            gauges: 2
        }
    ));
}

#[tokio::test]
async fn a_short_acquire_reply_is_an_error() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[1, 1])]);
    let error = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect_err("a short over-limit reply");
    assert!(matches!(
        error,
        GaugeError::ShortAcquireReply {
            needed: 3,
            length: 2
        }
    ));
}

#[tokio::test]
async fn an_empty_acquire_reply_is_an_error() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(Arc::clone(&cache), vec![int_values(&[])]);
    let error = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect_err("an empty acquire reply");
    assert!(matches!(error, GaugeError::MissingAcquireReply));
}

#[tokio::test]
async fn an_acquire_reply_that_is_not_an_array_is_an_error() {
    let cache = RecordingCache::new();
    let (gauges, _script) = acquire_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Element(CacheCounterValue::Int(0))],
    );
    let error = check_acquire(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]), "slot-a")
        .await
        .expect_err("a malformed acquire reply");
    assert!(matches!(error, GaugeError::Reply(_)));
}
