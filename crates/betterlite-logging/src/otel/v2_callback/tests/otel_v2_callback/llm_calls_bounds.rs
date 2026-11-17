#![cfg(test)]

use betterlite_logging::logging_pre_post::hooks::{ApiCallLogger as _, PreCallEvent};
use betterlite_logging::otel::logger::v2::OPEN_CALLS_MAX;
use betterlite_models::decisions::Json;

use super::support::{Harness, harness, settings};

fn store_calls(harness: &Harness, count: usize) {
    let messages = Json::Null;
    let model = "gpt-4o";
    for index in 0..count {
        let details: Json = serde_json::from_value(serde_json::json!({
            "litellm_call_id": index.to_string(),
            "model": model,
        }))
        .expect("details");
        let event = PreCallEvent {
            model,
            messages: &messages,
            details: &details,
        };
        harness.callback.log_pre_api_call(&event).expect("pre call");
    }
}

fn open_ids(harness: &Harness) -> Vec<String> {
    harness
        .callback
        .logger
        .open_calls()
        .calls
        .keys()
        .cloned()
        .collect()
}

#[test]
fn the_map_holds_exactly_the_cap() {
    let harness = harness(settings());
    store_calls(&harness, OPEN_CALLS_MAX);
    let ids = open_ids(&harness);
    assert_eq!(ids.len(), OPEN_CALLS_MAX);
    assert_eq!(ids.first().map(String::as_str), Some("0"));
}

#[test]
fn the_oldest_call_is_evicted_past_the_cap() {
    let harness = harness(settings());
    store_calls(&harness, OPEN_CALLS_MAX.saturating_add(1));
    let ids = open_ids(&harness);
    assert_eq!(ids.len(), OPEN_CALLS_MAX);
    assert!(!ids.contains(&"0".to_owned()));
    assert_eq!(ids.first().map(String::as_str), Some("1"));
    let last = OPEN_CALLS_MAX.to_string();
    assert_eq!(ids.last(), Some(&last));
}
