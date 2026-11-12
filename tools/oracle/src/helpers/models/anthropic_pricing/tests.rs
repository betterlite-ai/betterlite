#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use serde_json::json;

fn input(value: serde_json::Value) -> Input {
    serde_json::from_value(value).expect("a fixture input parses")
}

#[test]
fn token_pricing_applies_speed_before_geo_to_the_generic_totals() {
    let request = input(json!({
        "kind": "token",
        "model": "claude-test",
        "usage": {"prompt_tokens": 4_i64, "completion_tokens": 2_i64, "total_tokens": 6_i64},
        "model_info": {"input_cost_per_token": 0.25_f64, "output_cost_per_token": 0.5_f64},
        "provider_specific_entry": {"us": 1.1_f64, "fast": 2.0_f64},
        "service_tier": null,
        "inference_geo": "us",
        "speed": "fast",
    }));
    let output = Op::run(&request).expect("token pricing succeeds");
    assert_eq!(output["prompt_cost"], "2.2");
    assert_eq!(output["completion_cost"], "2.2");
}

#[test]
fn raw_probe_reports_each_response_count() {
    let request = input(json!({
        "kind": "raw_probe",
        "responses": [
            {"usage": {"server_tool_use": {"web_search_requests": "3"}}},
            {"usage": {}},
            [],
        ],
    }));
    let output = Op::run(&request).expect("the probe succeeds");
    assert_eq!(output, json!([3_i64, null, null]));
}

#[test]
fn accessor_returns_the_unvalidated_mapping_counter() {
    let request = input(json!({
        "kind": "web_search_requests",
        "record": {"kind": "mapping", "value": {"web_search_requests": "3"}},
    }));
    let output = Op::run(&request).expect("the accessor succeeds");
    assert_eq!(output, json!("3"));
}

#[test]
fn accessor_preserves_a_reported_zero_from_a_parsed_record() {
    let request = input(json!({
        "kind": "web_search_requests",
        "record": {"kind": "attribute", "value": {"web_search_requests": 0_i64}},
    }));
    let output = Op::run(&request).expect("the accessor succeeds");
    assert_eq!(output, json!(0_i64));
}

#[test]
fn search_cost_multiplies_the_medium_rate() {
    let request = input(json!({
        "kind": "search_cost",
        "model_info": {"input_cost_per_token": 0.25_f64},
        "search_context": {"search_context_size_medium": 0.02_f64},
        "record": {"kind": "mapping", "value": {"web_search_requests": 2_i64}},
    }));
    let output = Op::run(&request).expect("search pricing succeeds");
    assert_eq!(output["cost"], "0.04");
}
