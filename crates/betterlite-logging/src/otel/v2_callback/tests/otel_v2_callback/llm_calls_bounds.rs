#![cfg(test)]

use std::time::SystemTime;

use betterlite_logging::otel::logger::v2::{OPEN_CALLS_MAX, OpenCall};

use super::support::{harness, settings};

fn call() -> OpenCall {
    OpenCall {
        boundary: None,
        start_time: SystemTime::now(),
        provider: None,
    }
}

#[test]
fn the_map_holds_exactly_the_cap() {
    let harness = harness(settings());
    for index in 0..OPEN_CALLS_MAX {
        harness.callback.store_open_call(index.to_string(), call());
    }
    let ids = harness.callback.open_call_ids();
    assert_eq!(ids.len(), OPEN_CALLS_MAX);
    assert_eq!(ids.first().map(String::as_str), Some("0"));
}

#[test]
fn the_oldest_call_is_evicted_past_the_cap() {
    let harness = harness(settings());
    for index in 0..=OPEN_CALLS_MAX {
        harness.callback.store_open_call(index.to_string(), call());
    }
    let ids = harness.callback.open_call_ids();
    assert_eq!(ids.len(), OPEN_CALLS_MAX);
    assert!(!ids.contains(&"0".to_owned()));
    assert_eq!(ids.first().map(String::as_str), Some("1"));
    let last = OPEN_CALLS_MAX.to_string();
    assert_eq!(ids.last(), Some(&last));
}

#[test]
fn a_repeated_call_id_replaces_its_carrier() {
    let harness = harness(settings());
    harness
        .callback
        .store_open_call("call-1".to_owned(), call());
    harness
        .callback
        .store_open_call("call-1".to_owned(), call());
    assert_eq!(harness.callback.open_call_ids(), vec!["call-1".to_owned()]);
}
