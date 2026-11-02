#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use serde_json::json;

fn input(value: serde_json::Value) -> Input {
    serde_json::from_value(value).expect("a fixture input parses")
}

#[test]
fn audio_cost_per_second_a_strictly_positive_output_rate_charges_completion() {
    let request = input(json!({
        "operation": "audio_cost_per_second",
        "model": "fixture-speech",
        "custom_llm_provider": null,
        "duration": "4",
        "catalog": {"output_cost_per_second": "0.5", "input_cost_per_second": "0.1"},
    }));
    let output = Op::run(&request).expect("audio cost succeeds");
    assert_eq!(output["prompt_cost"], "0");
    assert_eq!(output["completion_cost"], "2");
}

#[test]
fn audio_cost_per_second_a_present_negative_input_rate_still_charges_prompt() {
    let request = input(json!({
        "operation": "audio_cost_per_second",
        "model": "fixture-speech",
        "custom_llm_provider": null,
        "duration": "4",
        "catalog": {"input_cost_per_second": "-0.5"},
    }));
    let output = Op::run(&request).expect("audio cost succeeds");
    assert_eq!(output["prompt_cost"], "-2");
    assert_eq!(output["completion_cost"], "0");
}

#[test]
fn wall_clock_per_second_an_empty_tiered_table_still_declines() {
    let request = input(json!({
        "operation": "wall_clock_per_second",
        "model": "fixture-realtime",
        "custom_llm_provider": null,
        "response_time_ms": "500",
        "catalog": {
            "input_cost_per_token": 0_i64,
            "output_cost_per_token": 0_i64,
            "tiered_pricing": [],
            "cost_per_second": "0.2",
        },
    }));
    let output = Op::run(&request).expect("wall clock lookup succeeds");
    assert_eq!(output["charged"], false);
}

#[test]
fn wall_clock_per_second_zero_token_prices_with_no_tiered_table_charges_the_input_side() {
    let request = input(json!({
        "operation": "wall_clock_per_second",
        "model": "fixture-realtime",
        "custom_llm_provider": null,
        "response_time_ms": "500",
        "catalog": {
            "input_cost_per_token": 0_i64,
            "output_cost_per_token": 0_i64,
            "cost_per_second": "0.2",
        },
    }));
    let output = Op::run(&request).expect("wall clock lookup succeeds");
    assert_eq!(output["charged"], true);
    assert_eq!(output["prompt_cost"], "0.1");
    assert_eq!(output["completion_cost"], "0");
}

#[test]
fn video_a_supplied_empty_price_table_bypasses_the_public_catalog_and_prices_zero() {
    let request = input(json!({
        "operation": "video",
        "calculator": "default",
        "model": "fixture-video",
        "duration_seconds": "10",
        "custom_llm_provider": null,
        "model_info": {},
        "video_resolution": "1080p",
        "raw_catalog": {"output_cost_per_second": "9"},
    }));
    let output = Op::run(&request).expect("video cost succeeds");
    assert_eq!(output["cost"], "0");
}

#[test]
fn video_a_free_per_video_second_rate_wins_over_a_resolution_specific_rate() {
    let request = input(json!({
        "operation": "video",
        "calculator": "openai",
        "model": "fixture-video",
        "duration_seconds": "10",
        "custom_llm_provider": null,
        "model_info": {
            "output_cost_per_video_per_second": 0_i64,
            "output_cost_per_second_1080p": "5",
        },
        "video_resolution": "1,080p",
        "raw_catalog": null,
    }));
    let output = Op::run(&request).expect("video cost succeeds");
    assert_eq!(output["cost"], "0");
}

#[test]
fn character_metric_a_present_character_rate_wins_and_128k_is_a_strict_boundary() {
    let request = input(json!({
        "operation": "character_metric",
        "key": "fixture-model",
        "record": {"input_cost_per_character": "0.002", "input_cost_per_token": "0.01"},
        "above_128k_tokens": ["128000", "128000.0001", "127999.9999"],
    }));
    let output = Op::run(&request).expect("character metric succeeds");
    assert_eq!(output["metric"], "cost_per_character");
    assert_eq!(output["above_128k"], json!([false, true, false]));
}
