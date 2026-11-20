#![cfg(test)]

use betterlite_oracle_helper_api::helper::Helper as _;
use serde_json::json;

use super::{Input, Op};

#[test]
fn track_end_user_logs_the_removal_of_the_oldest_series() {
    let input = json!({
        "function": "track_end_user",
        "metric_name": "litellm_requests_metric",
        "supported_labels": ["model", "end_user"],
        "metrics": {"probe": "removed"},
        "steps": [
            {"metric": "probe", "labels": {"model": "m", "end_user": "e1"}, "at_seconds": 0, "max_series": 1},
            {"metric": "probe", "labels": {"model": "m", "end_user": "e2"}, "at_seconds": 1, "max_series": 1}
        ]
    });
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    let output = Op::run(&parsed).unwrap();
    assert_eq!(
        output,
        json!({"events": [{"event": "removal", "metric": "probe", "labels": ["m", "e1"]}]})
    );
}

#[test]
fn inc_labeled_counter_logs_the_increment_before_the_removal() {
    let input = json!({
        "function": "inc_labeled_counter",
        "metric_name": "litellm_requests_metric",
        "supported_labels": ["model", "end_user"],
        "enable_end_user_tracking": true,
        "max_series": 1,
        "amount": 2,
        "emissions": [
            {"values": {"model": "m", "end_user": "e1"}, "at_seconds": 0},
            {"values": {"model": "m", "end_user": "e2"}, "at_seconds": 1}
        ]
    });
    let parsed = serde_json::from_value::<Input>(input).unwrap();
    let output = Op::run(&parsed).unwrap();
    assert_eq!(
        output,
        json!({"events": [
            {"event": "increment", "metric": "litellm_requests_metric", "labels": {"end_user": "e1", "model": "m"}, "amount": "2"},
            {"event": "increment", "metric": "litellm_requests_metric", "labels": {"end_user": "e2", "model": "m"}, "amount": "2"},
            {"event": "removal", "metric": "litellm_requests_metric", "labels": ["m", "e1"]}
        ]})
    );
}
