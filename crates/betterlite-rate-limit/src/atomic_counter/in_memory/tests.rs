#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, RateLimitResponse, RateLimitStatus, RateLimitType,
    ReservationBackend, ReservationWindowIdentity, WindowRateLimitType,
};

use super::{LocalCounterCache, atomic_check_and_increment_in_memory};
use crate::atomic_counter::value::CounterValueError;

const NOW: i64 = 1_700_000_300;
const WINDOW_KEY: &str = "{model:gpt-4}:window";
const REQUESTS_KEY: &str = "{model:gpt-4}:requests";
const TOKENS_KEY: &str = "{model:gpt-4}:tokens";

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReadCall {
    key: String,
    local_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct WriteCall {
    key: String,
    value: CacheCounterValue,
    ttl: i64,
    local_only: bool,
}

#[derive(Default)]
struct RecordingState {
    values: BTreeMap<String, CacheCounterValue>,
    reads: Vec<ReadCall>,
    writes: Vec<WriteCall>,
}

struct RecordingCache(Mutex<RecordingState>);

impl RecordingCache {
    fn new(entries: &[(&str, CacheCounterValue)]) -> Self {
        let values = entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect();
        Self(Mutex::new(RecordingState {
            values,
            reads: Vec::new(),
            writes: Vec::new(),
        }))
    }

    fn reads(&self) -> Vec<ReadCall> {
        self.0.lock().unwrap().reads.clone()
    }

    fn writes(&self) -> Vec<WriteCall> {
        self.0.lock().unwrap().writes.clone()
    }
}

#[async_trait::async_trait]
impl LocalCounterCache for RecordingCache {
    async fn get(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        let mut state = self.0.lock().unwrap();
        state.reads.push(ReadCall {
            key: key.to_owned(),
            local_only,
        });
        state.values.get(key).cloned()
    }

    async fn set(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        let mut state = self.0.lock().unwrap();
        state.writes.push(WriteCall {
            key: key.to_owned(),
            value: value.clone(),
            ttl,
            local_only,
        });
        state.values.insert(key.to_owned(), value);
    }
}

fn meta(
    rate_limit_type: WindowRateLimitType,
    current_limit: i64,
    increment: i64,
    ttl: i64,
    window_size: i64,
) -> AtomicCounterMeta {
    let counter_key = match rate_limit_type {
        WindowRateLimitType::Requests => REQUESTS_KEY,
        WindowRateLimitType::Tokens => TOKENS_KEY,
    };
    AtomicCounterMeta {
        descriptor_key: "model".to_owned(),
        descriptor_value: "gpt-4".to_owned(),
        current_limit,
        rate_limit_type,
        window_key: WINDOW_KEY.to_owned(),
        counter_key: counter_key.to_owned(),
        increment,
        ttl,
        window_size,
    }
}

fn counter(
    rate_limit_type: WindowRateLimitType,
    current_limit: i64,
    increment: i64,
) -> AtomicCounterMeta {
    meta(rate_limit_type, current_limit, increment, 60, 60)
}

fn read(key: &str) -> ReadCall {
    ReadCall {
        key: key.to_owned(),
        local_only: true,
    }
}

fn written(key: &str, value: CacheCounterValue, ttl: i64) -> WriteCall {
    WriteCall {
        key: key.to_owned(),
        value,
        ttl,
        local_only: true,
    }
}

fn status(
    code: &str,
    current_limit: i64,
    limit_remaining: i64,
    rate_limit_type: RateLimitType,
) -> RateLimitStatus {
    RateLimitStatus {
        code: code.to_owned(),
        current_limit,
        limit_remaining,
        rate_limit_type,
        descriptor_key: "model".to_owned(),
        descriptor_value: Some("gpt-4".to_owned()),
    }
}

fn window(counter_key: &str, window_start: &str) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        backend: ReservationBackend::Local,
    }
}

fn window_start_text(seconds: i64) -> CacheCounterValue {
    CacheCounterValue::Text(seconds.to_string())
}

#[tokio::test]
async fn in_memory_increments_an_active_window() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(7)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(cache.reads(), vec![read(WINDOW_KEY), read(REQUESTS_KEY)]);
    assert_eq!(
        cache.writes(),
        vec![written(REQUESTS_KEY, CacheCounterValue::Int(8), 60)]
    );
    assert_eq!(
        response,
        RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: vec![status("OK", 10, 2, RateLimitType::Requests)],
            reservation_windows: Some(BTreeSet::from([window(REQUESTS_KEY, "1700000299")])),
        }
    );
}

#[tokio::test]
async fn in_memory_resets_a_missing_window_without_reading_the_counter() {
    let cache = RecordingCache::new(&[]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(cache.reads(), vec![read(WINDOW_KEY)]);
    assert_eq!(
        cache.writes(),
        vec![
            written(REQUESTS_KEY, CacheCounterValue::Int(0), 60),
            written(TOKENS_KEY, CacheCounterValue::Int(0), 60),
            written(WINDOW_KEY, window_start_text(NOW), 60),
            written(REQUESTS_KEY, CacheCounterValue::Int(1), 60),
        ]
    );
    assert_eq!(
        response.reservation_windows,
        Some(BTreeSet::from([window(REQUESTS_KEY, "1700000300")]))
    );
}

#[tokio::test]
async fn in_memory_expires_a_window_at_exactly_the_window_size() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(NOW - 60)),
        (REQUESTS_KEY, CacheCounterValue::Int(12)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(cache.reads(), vec![read(WINDOW_KEY)]);
    assert_eq!(
        cache.writes(),
        vec![
            written(REQUESTS_KEY, CacheCounterValue::Int(0), 60),
            written(TOKENS_KEY, CacheCounterValue::Int(0), 60),
            written(WINDOW_KEY, window_start_text(NOW), 60),
            written(REQUESTS_KEY, CacheCounterValue::Int(1), 60),
        ]
    );
    assert_eq!(response.statuses[0].limit_remaining, 9);
    assert_eq!(
        response.reservation_windows,
        Some(BTreeSet::from([window(REQUESTS_KEY, "1700000300")]))
    );
}

#[tokio::test]
async fn in_memory_keeps_a_window_one_second_inside_the_window_size() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(NOW - 59)),
        (REQUESTS_KEY, CacheCounterValue::Int(3)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(cache.reads(), vec![read(WINDOW_KEY), read(REQUESTS_KEY)]);
    assert_eq!(
        cache.writes(),
        vec![written(REQUESTS_KEY, CacheCounterValue::Int(4), 60)]
    );
    assert_eq!(
        response.reservation_windows,
        Some(BTreeSet::from([window(REQUESTS_KEY, "1700000241")]))
    );
}

#[tokio::test]
async fn in_memory_reads_a_window_start_stored_as_an_integer() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, CacheCounterValue::Int(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(7)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        response.reservation_windows,
        Some(BTreeSet::from([window(REQUESTS_KEY, "1700000299")]))
    );
}

#[tokio::test]
async fn in_memory_resets_a_shared_window_and_its_siblings_once() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_100)),
        (REQUESTS_KEY, CacheCounterValue::Int(3)),
        (TOKENS_KEY, CacheCounterValue::Int(50)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[
            counter(WindowRateLimitType::Requests, 10, 1),
            counter(WindowRateLimitType::Tokens, 200, 5),
        ],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(cache.reads(), vec![read(WINDOW_KEY), read(WINDOW_KEY)]);
    assert_eq!(
        cache.writes(),
        vec![
            written(REQUESTS_KEY, CacheCounterValue::Int(0), 60),
            written(TOKENS_KEY, CacheCounterValue::Int(0), 60),
            written(WINDOW_KEY, window_start_text(NOW), 60),
            written(REQUESTS_KEY, CacheCounterValue::Int(1), 60),
            written(WINDOW_KEY, window_start_text(NOW), 60),
            written(TOKENS_KEY, CacheCounterValue::Int(5), 60),
        ]
    );
    assert_eq!(
        response.statuses,
        vec![
            status("OK", 10, 9, RateLimitType::Requests),
            status("OK", 200, 195, RateLimitType::Tokens),
        ]
    );
}

#[tokio::test]
async fn in_memory_writes_the_window_size_ttl_for_siblings_and_the_counter_ttl_for_counters() {
    let cache = RecordingCache::new(&[(WINDOW_KEY, window_start_text(1_700_000_100))]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[meta(WindowRateLimitType::Requests, 10, 1, 120, 30)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(response.statuses[0].limit_remaining, 9);
    assert_eq!(
        cache.writes(),
        vec![
            written(REQUESTS_KEY, CacheCounterValue::Int(0), 30),
            written(TOKENS_KEY, CacheCounterValue::Int(0), 30),
            written(WINDOW_KEY, window_start_text(NOW), 30),
            written(REQUESTS_KEY, CacheCounterValue::Int(1), 120),
        ]
    );
}

#[tokio::test]
async fn in_memory_blocks_the_whole_call_when_a_later_counter_is_over_its_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(9)),
        (TOKENS_KEY, CacheCounterValue::Int(200)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[
            counter(WindowRateLimitType::Requests, 10, 1),
            counter(WindowRateLimitType::Tokens, 200, 5),
        ],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(cache.reads().len(), 4);
    assert!(cache.writes().is_empty());
    assert_eq!(
        response,
        RateLimitResponse {
            overall_code: "OVER_LIMIT".to_owned(),
            statuses: vec![status("OVER_LIMIT", 200, 0, RateLimitType::Tokens)],
            reservation_windows: None,
        }
    );
}

#[tokio::test]
async fn in_memory_allows_an_increment_that_reaches_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(9)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        cache.writes(),
        vec![written(REQUESTS_KEY, CacheCounterValue::Int(10), 60)]
    );
    assert_eq!(response.statuses[0].limit_remaining, 0);
}

#[tokio::test]
async fn in_memory_blocks_a_positive_increment_that_would_cross_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(9)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 5)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(
        response.statuses,
        vec![status("OVER_LIMIT", 10, 1, RateLimitType::Requests)]
    );
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_blocks_a_positive_increment_past_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(10)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(
        response.statuses,
        vec![status("OVER_LIMIT", 10, 0, RateLimitType::Requests)]
    );
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_blocks_a_zero_increment_at_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(10)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 0)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_allows_a_zero_increment_below_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(9)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, 0)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        cache.writes(),
        vec![written(REQUESTS_KEY, CacheCounterValue::Int(9), 60)]
    );
    assert_eq!(response.statuses[0].limit_remaining, 1);
}

#[tokio::test]
async fn in_memory_blocks_a_negative_increment_at_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(10)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, -1)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_refunds_a_negative_increment_below_the_limit() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Int(5)),
    ]);
    let response = atomic_check_and_increment_in_memory(
        &cache,
        &[counter(WindowRateLimitType::Requests, 10, -2)],
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        cache.writes(),
        vec![written(REQUESTS_KEY, CacheCounterValue::Int(3), 60)]
    );
    assert_eq!(response.statuses[0].limit_remaining, 7);
}

#[tokio::test]
async fn in_memory_rejects_a_non_integer_window_start() {
    let cache = RecordingCache::new(&[(WINDOW_KEY, CacheCounterValue::Text("soon".to_owned()))]);
    assert_eq!(
        atomic_check_and_increment_in_memory(
            &cache,
            &[counter(WindowRateLimitType::Requests, 10, 1)],
            NOW
        )
        .await,
        Err(CounterValueError::NotAnInteger {
            position: WINDOW_KEY.to_owned(),
            value: CacheCounterValue::Text("soon".to_owned()),
        })
    );
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_rejects_a_non_integer_counter() {
    let cache = RecordingCache::new(&[
        (WINDOW_KEY, window_start_text(1_700_000_299)),
        (REQUESTS_KEY, CacheCounterValue::Text("many".to_owned())),
    ]);
    assert_eq!(
        atomic_check_and_increment_in_memory(
            &cache,
            &[counter(WindowRateLimitType::Requests, 10, 1)],
            NOW
        )
        .await,
        Err(CounterValueError::NotAnInteger {
            position: REQUESTS_KEY.to_owned(),
            value: CacheCounterValue::Text("many".to_owned()),
        })
    );
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn in_memory_returns_an_empty_success_without_counters() {
    let cache = RecordingCache::new(&[]);
    let response = atomic_check_and_increment_in_memory(&cache, &[], NOW)
        .await
        .unwrap();
    assert_eq!(
        response,
        RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: Vec::new(),
            reservation_windows: Some(BTreeSet::new()),
        }
    );
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}
