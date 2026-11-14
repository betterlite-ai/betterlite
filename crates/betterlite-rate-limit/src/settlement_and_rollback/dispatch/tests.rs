#![cfg(test)]

use std::sync::Arc;

use crate::limiter_records::{CacheCounterValue, RateLimitType};
use crate::settlement_and_rollback::SettlementError;
use crate::settlement_and_rollback::tests::support::{
    CacheRead, CacheWrite, FakeCache, FakeScript, FixedClock, amounts, descriptor, reply,
    settlement, store, text,
};

const NOW: u64 = 1_700_000_000;

#[tokio::test]
async fn a_configured_script_serves_the_lua_path() {
    let script = FakeScript::new(vec![reply(&[0, 4, 100])]);
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, Some(Arc::clone(&script)), None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the script settles the descriptor");
    let calls = script.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls.first().map(|call| call.args.clone()),
        Some(vec![text("10"), text("1"), text("60"), text("60")])
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    assert_eq!(
        response
            .statuses
            .first()
            .map(|status| status.limit_remaining),
        Some(6)
    );
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn without_a_script_the_memory_store_starts_the_window() {
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, None, None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the in-memory path settles the descriptor");
    assert_eq!(
        cache.reads(),
        vec![CacheRead {
            key: "{model:gpt-4}:window".to_owned(),
            local_only: true,
        }]
    );
    assert_eq!(
        cache.writes(),
        vec![
            CacheWrite {
                key: "{model:gpt-4}:requests".to_owned(),
                value: CacheCounterValue::Int(0),
                ttl: 60,
                local_only: true,
            },
            CacheWrite {
                key: "{model:gpt-4}:tokens".to_owned(),
                value: CacheCounterValue::Int(0),
                ttl: 60,
                local_only: true,
            },
            CacheWrite {
                key: "{model:gpt-4}:window".to_owned(),
                value: CacheCounterValue::Text(NOW.to_string()),
                ttl: 60,
                local_only: true,
            },
            CacheWrite {
                key: "{model:gpt-4}:requests".to_owned(),
                value: CacheCounterValue::Int(1),
                ttl: 60,
                local_only: true,
            },
        ]
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses.len(), 1);
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.rate_limit_type, RateLimitType::Requests);
    assert_eq!(status.limit_remaining, 9);
}

#[tokio::test]
async fn the_memory_path_reports_an_over_limit_without_writing() {
    let cache = FakeCache::seeded(&[
        (
            "{model:gpt-4}:window",
            CacheCounterValue::Text(NOW.to_string()),
        ),
        ("{model:gpt-4}:requests", CacheCounterValue::Int(10)),
    ]);
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::at(NOW);
    let settler = settlement(&windows, &clock, None, None, false);
    let response = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect("the in-memory path reports the limit");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one status");
    assert_eq!(status.current_limit, 10);
    assert_eq!(status.limit_remaining, 0);
    assert!(cache.writes().is_empty());
    let reads = cache.reads();
    let read_keys: Vec<&str> = reads.iter().map(|read| read.key.as_str()).collect();
    assert_eq!(
        read_keys,
        vec!["{model:gpt-4}:window", "{model:gpt-4}:requests"]
    );
}

#[tokio::test]
async fn a_clock_before_the_epoch_is_an_error() {
    let cache = FakeCache::empty();
    let windows = store(Arc::clone(&cache));
    let clock = FixedClock::before_epoch();
    let settler = settlement(&windows, &clock, None, None, false);
    let error = settler
        .atomic_check_and_increment_by_n(
            &[descriptor("model", "gpt-4", Some(10), None)],
            &[amounts(1, 0)],
        )
        .await
        .expect_err("a pre-epoch clock has no window start");
    assert!(matches!(error, SettlementError::ClockBeforeEpoch(_)));
    assert!(cache.writes().is_empty());
}
