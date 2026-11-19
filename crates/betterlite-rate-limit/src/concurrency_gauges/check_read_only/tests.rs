#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::concurrency_gauges::tests::support::{
    FixedClock, LogCapture, PlannedScript, RecordingCache, ReplyPlan, gauge, gauges_with,
    int_counter, int_values, key_list,
};
use crate::concurrency_gauges::{ConcurrencyGauges, GaugeError};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::CacheCounterValue;
use crate::rate_limit_window_store::CounterScript;

use super::check_read_only;

const NOW: f64 = 1_700_000_000.0;

fn count_gauges(
    cache: Arc<RecordingCache>,
    plans: Vec<ReplyPlan>,
) -> (ConcurrencyGauges, Arc<PlannedScript>) {
    let mut gauges = gauges_with(cache, FixedClock::at(NOW));
    let handle = PlannedScript::new("count", plans);
    let script: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(&handle);
    gauges.count_script = Some(script);
    (gauges, handle)
}

#[tokio::test]
async fn redis_success_returns_the_script_counts() {
    let cache = RecordingCache::new();
    let (gauges, script) = count_gauges(Arc::clone(&cache), vec![int_values(&[3])]);
    let response = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect("a read-only gauge response");
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.code, "OK");
    assert_eq!(status.limit_remaining, 2);
    assert_eq!(
        script.calls(),
        vec![(
            key_list(&["g1"]),
            vec![RedisScalar::Text("3600".to_owned())],
        )]
    );
    assert!(cache.batch_reads().is_empty());
}

#[tokio::test]
async fn redis_success_reports_each_gauge_in_order() {
    let cache = RecordingCache::new();
    let (gauges, _script) = count_gauges(Arc::clone(&cache), vec![int_values(&[1, 4])]);
    let response = check_read_only(
        &gauges,
        &[gauge("g1", 2), gauge("g2", 4)],
        &key_list(&["g1", "g2"]),
    )
    .await
    .expect("a read-only gauge response");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let statuses = response.statuses;
    assert_eq!(statuses.len(), 2);
    let first = statuses.first().expect("the first gauge status");
    let second = statuses.get(1).expect("the second gauge status");
    assert_eq!(first.code, "OK");
    assert_eq!(first.limit_remaining, 1);
    assert_eq!(second.code, "OVER_LIMIT");
    assert_eq!(second.limit_remaining, 0);
}

#[tokio::test]
async fn redis_success_negative_counts_read_as_zero() {
    let cache = RecordingCache::new();
    let (gauges, _script) = count_gauges(Arc::clone(&cache), vec![int_values(&[-3])]);
    let response = check_read_only(&gauges, &[gauge("g1", 2)], &key_list(&["g1"]))
        .await
        .expect("a read-only gauge response");
    assert_eq!(response.overall_code, "OK");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 2);
}

#[tokio::test]
async fn redis_failure_fallback_to_the_local_mirror() {
    let capture = LogCapture::start();
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    let (gauges, _script) = count_gauges(Arc::clone(&cache), vec![ReplyPlan::Failure]);
    let response = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect("a local gauge response");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 3);
    assert_eq!(cache.batch_reads().len(), 1);
    assert!(capture.text().contains("parallel_count_script failed"));
}

#[tokio::test]
async fn script_absent_fallback_to_local_values() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(1));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect("a local gauge response");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 4);
    assert_eq!(cache.batch_reads().len(), 1);
}

#[tokio::test]
async fn local_fallback_honours_the_over_limit_check() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = check_read_only(&gauges, &[gauge("g1", 2)], &key_list(&["g1"]))
        .await
        .expect("a local gauge response");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.code, "OVER_LIMIT");
}

#[tokio::test]
async fn redis_failure_rejects_when_fail_closed() {
    let cache = RecordingCache::new();
    let (mut gauges, _script) = count_gauges(Arc::clone(&cache), vec![ReplyPlan::Failure]);
    gauges.fail_closed = FailClosedPolicy { enabled: true };
    let error = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect_err("a fail-closed rejection");
    assert!(matches!(
        &error,
        GaugeError::Unverifiable(rejection) if rejection.status_code == 503
    ));
    assert!(cache.batch_reads().is_empty());
}

#[tokio::test]
async fn a_count_reply_that_is_not_an_array_falls_back_to_the_local_mirror() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(1));
    let (gauges, _script) = count_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Element(CacheCounterValue::Int(0))],
    );
    let response = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect("a local gauge response");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 4);
    assert_eq!(cache.batch_reads().len(), 1);
}

#[tokio::test]
async fn a_count_that_is_not_whole_falls_back_to_the_local_mirror() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    let (gauges, _script) = count_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Values(vec![CacheCounterValue::Text(
            "soon".to_owned(),
        )])],
    );
    let response = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect("a local gauge response");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 3);
}

#[tokio::test]
async fn a_malformed_count_reply_rejects_when_fail_closed() {
    let cache = RecordingCache::new();
    let (mut gauges, _script) = count_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Element(CacheCounterValue::Int(0))],
    );
    gauges.fail_closed = FailClosedPolicy { enabled: true };
    let error = check_read_only(&gauges, &[gauge("g1", 5)], &key_list(&["g1"]))
        .await
        .expect_err("a fail-closed rejection");
    assert!(matches!(
        &error,
        GaugeError::Unverifiable(rejection) if rejection.status_code == 503
    ));
}
