#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use serde_json::json;

fn input(value: serde_json::Value) -> Input {
    serde_json::from_value(value).expect("a fixture input parses")
}

#[test]
fn a_raw_zero_count_projects_a_repaired_usage() {
    let request = input(json!({
        "responses": [
            {"raw": {"body": {"usage": {"server_tool_use": {"web_search_requests": 0}}}, "usage": null}},
        ],
    }));
    let output = Op::run(&request).expect("the probes succeed");
    assert_eq!(output["responses"][0]["web_search"], json!(true));
    assert_eq!(output["responses"][0]["file_search"], json!(false));
    assert_eq!(output["responses"][0]["count"], json!(1_i64));
    assert_eq!(
        output["responses"][0]["usage"]["server_tool_use"]["web_search_requests"],
        json!(0_i64)
    );
}

#[test]
fn a_responses_reported_count_beats_its_items() {
    let request = input(json!({
        "responses": [
            {"responses": {
                "output": [{"mapping": {"type": "web_search_call"}}, {"mapping": {"type": "web_search_call"}}],
                "tool_usage": {"web_search": {"num_requests": 0}},
                "usage": null,
            }},
        ],
    }));
    let output = Op::run(&request).expect("the probes succeed");
    assert_eq!(output["responses"][0]["count"], json!(0_i64));
    assert_eq!(output["responses"][0]["web_search"], json!(true));
}

#[test]
fn kwargs_outputs_copy_every_field() {
    let request = input(json!({
        "kwargs": [
            {"web_search_options": {"search_context_size": "gigantic", "extra": 1}},
            {"tools": [{"type": "file_search", "vector_store_ids": ["vs_1"], "extra": true}]},
        ],
    }));
    let output = Op::run(&request).expect("the tool selection succeeds");
    assert_eq!(
        output["kwargs"][0]["web_search_options"]["search_context_size"],
        json!("gigantic")
    );
    assert_eq!(
        output["kwargs"][0]["web_search_options"]["extra"],
        json!(1_i64)
    );
    assert_eq!(output["kwargs"][1]["web_search_options"], json!(null));
    assert_eq!(
        output["kwargs"][1]["file_search"]["vector_store_ids"],
        json!(["vs_1"])
    );
    assert_eq!(output["kwargs"][1]["file_search"]["extra"], json!(true));
}

#[test]
fn conversions_project_strings_and_counts() {
    let request = input(json!({
        "file_search_params": [{"storage_gb": 2.5, "days": "3"}, {"storage_gb": "abc", "days": null}],
        "token_counts": [{"input_tokens": 3.9, "output_tokens": "7"}],
        "values": [3, " 1_0 ", "3.0"],
    }));
    let output = Op::run(&request).expect("the conversions succeed");
    assert_eq!(
        output["file_search_params"],
        json!([["2.5", "3"], [null, null]])
    );
    assert_eq!(output["token_counts"], json!([[3_i64, 7_i64]]));
    assert_eq!(output["values"], json!([3_i64, 10_i64, null]));
}

#[test]
fn explicit_null_options_fail_the_adapter() {
    let request = input(json!({"kwargs": [{"web_search_options": null}]}));
    let error = Op::run(&request).expect_err("a null options value fails");
    assert_eq!(error.class, "ToolSignalError");
}

#[test]
fn an_empty_input_projects_an_empty_report() {
    let request = input(json!({}));
    let output = Op::run(&request).expect("an empty input succeeds");
    assert_eq!(
        output,
        json!({
            "responses": [],
            "file_search_params": [],
            "token_counts": [],
            "values": [],
            "kwargs": [],
        })
    );
}
