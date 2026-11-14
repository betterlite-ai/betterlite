#![cfg(test)]

use std::collections::BTreeSet;

use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, RateLimitResponse, RateLimitStatus, RateLimitType,
    ReservationBackend, ReservationWindowIdentity, WindowRateLimitType,
};

use super::{ResponseDecodeError, build_atomic_response};
use crate::atomic_counter::value::CounterValueError;

fn meta(
    rate_limit_type: WindowRateLimitType,
    current_limit: i64,
    counter_key: &str,
) -> AtomicCounterMeta {
    AtomicCounterMeta {
        descriptor_key: "model".to_owned(),
        descriptor_value: "gpt-4".to_owned(),
        current_limit,
        rate_limit_type,
        window_key: "{model:gpt-4}:window".to_owned(),
        counter_key: counter_key.to_owned(),
        increment: 1,
        ttl: 60,
        window_size: 60,
    }
}

fn requests(current_limit: i64) -> AtomicCounterMeta {
    meta(
        WindowRateLimitType::Requests,
        current_limit,
        "{model:gpt-4}:requests",
    )
}

fn tokens(current_limit: i64) -> AtomicCounterMeta {
    meta(
        WindowRateLimitType::Tokens,
        current_limit,
        "{model:gpt-4}:tokens",
    )
}

fn identity(
    counter_key: &str,
    window_start: &str,
    backend: ReservationBackend,
) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        backend,
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

#[test]
fn response_decodes_a_success_reply_into_counter_statuses() {
    let raw = [
        CacheCounterValue::Int(0),
        CacheCounterValue::Int(4),
        CacheCounterValue::Int(1_700_000_100),
    ];
    let response = build_atomic_response(&raw, &[requests(10)]).unwrap();
    assert_eq!(
        response,
        RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: vec![status("OK", 10, 6, RateLimitType::Requests)],
            reservation_windows: Some(BTreeSet::from([identity(
                "{model:gpt-4}:requests",
                "1700000100",
                ReservationBackend::Redis,
            )])),
        }
    );
}

#[test]
fn response_reads_a_counter_and_a_window_start_for_each_counter() {
    let raw = [
        CacheCounterValue::Int(0),
        CacheCounterValue::Int(4),
        CacheCounterValue::Int(1_700_000_100),
        CacheCounterValue::Int(250),
        CacheCounterValue::Int(1_700_000_200),
    ];
    let response = build_atomic_response(&raw, &[requests(10), tokens(300)]).unwrap();
    assert_eq!(
        response.statuses,
        vec![
            status("OK", 10, 6, RateLimitType::Requests),
            status("OK", 300, 50, RateLimitType::Tokens),
        ]
    );
    assert_eq!(
        response.reservation_windows,
        Some(BTreeSet::from([
            identity(
                "{model:gpt-4}:requests",
                "1700000100",
                ReservationBackend::Redis,
            ),
            identity(
                "{model:gpt-4}:tokens",
                "1700000200",
                ReservationBackend::Redis,
            ),
        ]))
    );
}

#[test]
fn response_clamps_the_remaining_counter_at_zero() {
    let raw = [
        CacheCounterValue::Int(0),
        CacheCounterValue::Int(12),
        CacheCounterValue::Int(1_700_000_100),
    ];
    let response = build_atomic_response(&raw, &[requests(10)]).unwrap();
    assert_eq!(response.statuses[0].limit_remaining, 0);
}

#[test]
fn response_reads_text_and_byte_reply_values() {
    let text = [
        CacheCounterValue::Text("0".to_owned()),
        CacheCounterValue::Text("4".to_owned()),
        CacheCounterValue::Text("1700000100".to_owned()),
    ];
    let bytes = [
        CacheCounterValue::Bytes(b"0".to_vec()),
        CacheCounterValue::Bytes(b"4".to_vec()),
        CacheCounterValue::Bytes(b"1700000100".to_vec()),
    ];
    let from_text = build_atomic_response(&text, &[requests(10)]).unwrap();
    let from_bytes = build_atomic_response(&bytes, &[requests(10)]).unwrap();
    assert_eq!(from_text, from_bytes);
    assert_eq!(from_text.statuses[0].limit_remaining, 6);
}

#[test]
fn response_decodes_an_over_limit_reply() {
    let raw = [
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(12),
        CacheCounterValue::Int(10),
    ];
    let response = build_atomic_response(&raw, &[requests(10)]).unwrap();
    assert_eq!(
        response,
        RateLimitResponse {
            overall_code: "OVER_LIMIT".to_owned(),
            statuses: vec![status("OVER_LIMIT", 10, 0, RateLimitType::Requests)],
            reservation_windows: None,
        }
    );
}

#[test]
fn response_over_limit_keeps_the_remaining_counter() {
    let raw = [
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(4),
        CacheCounterValue::Int(10),
    ];
    let response = build_atomic_response(&raw, &[requests(10)]).unwrap();
    assert_eq!(response.statuses[0].limit_remaining, 6);
}

#[test]
fn response_over_limit_names_the_blocked_counter() {
    let raw = [
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(2),
        CacheCounterValue::Int(250),
        CacheCounterValue::Int(300),
    ];
    let response = build_atomic_response(&raw, &[requests(10), tokens(300)]).unwrap();
    assert_eq!(
        response.statuses,
        vec![status("OVER_LIMIT", 300, 50, RateLimitType::Tokens)]
    );
}

#[test]
fn response_returns_an_empty_success_for_an_empty_reply() {
    let response = build_atomic_response(&[], &[requests(10)]).unwrap();
    assert_eq!(
        response,
        RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: Vec::new(),
            reservation_windows: None,
        }
    );
}

#[test]
fn response_treats_a_status_other_than_one_as_success_like_litellm() {
    let raw = [
        CacheCounterValue::Int(2),
        CacheCounterValue::Int(4),
        CacheCounterValue::Int(1_700_000_100),
    ];
    let response = build_atomic_response(&raw, &[requests(10)]).unwrap();
    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses[0].limit_remaining, 6);
}

#[test]
fn response_rejects_a_reply_that_stops_before_a_counter() {
    assert_eq!(
        build_atomic_response(&[CacheCounterValue::Int(0)], &[requests(10)]),
        Err(ResponseDecodeError::MissingElement { position: 1 })
    );
}

#[test]
fn response_rejects_an_over_limit_reply_that_is_too_short() {
    assert_eq!(
        build_atomic_response(
            &[CacheCounterValue::Int(1), CacheCounterValue::Int(1)],
            &[requests(10)]
        ),
        Err(ResponseDecodeError::MissingElement { position: 2 })
    );
}

#[test]
fn response_rejects_a_counter_index_past_the_last_counter() {
    let raw = [
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(2),
        CacheCounterValue::Int(5),
        CacheCounterValue::Int(10),
    ];
    assert_eq!(
        build_atomic_response(&raw, &[requests(10)]),
        Err(ResponseDecodeError::CounterIndex { index: 2, count: 1 })
    );
}

#[test]
fn response_rejects_a_counter_index_below_one() {
    let raw = [
        CacheCounterValue::Int(1),
        CacheCounterValue::Int(0),
        CacheCounterValue::Int(5),
        CacheCounterValue::Int(10),
    ];
    assert_eq!(
        build_atomic_response(&raw, &[requests(10)]),
        Err(ResponseDecodeError::CounterIndex { index: 0, count: 1 })
    );
}

#[test]
fn response_rejects_a_non_integer_status_code() {
    assert_eq!(
        build_atomic_response(
            &[CacheCounterValue::Text("many".to_owned())],
            &[requests(10)]
        ),
        Err(ResponseDecodeError::Value(
            CounterValueError::NotAnInteger {
                position: "reply[0]".to_owned(),
                value: CacheCounterValue::Text("many".to_owned()),
            }
        ))
    );
}
