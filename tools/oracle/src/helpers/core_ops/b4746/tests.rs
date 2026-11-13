#![cfg(test)]

use serde_json::{Value, json};

use crate::helpers::Helper as _;

use super::Op;
use super::input::Input;

fn run(input: &Value) -> Value {
    Op::decode_and_run(input).unwrap()
}

fn input(operation: &str, extra: &Value) -> Value {
    let mut value = json!({
        "operation": operation,
        "client_timeout_ms": 60_000_u64,
        "listing_timeout_ms": 30_000_u64,
        "raise_on_error": false,
        "capabilities": null,
        "pages": [],
        "prompt": null,
        "resource": null
    });
    if let (Some(target), Some(source)) = (value.as_object_mut(), extra.as_object()) {
        target.extend(source.clone());
    }
    value
}

#[test]
fn prompts_collect_every_page_in_cursor_order() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": {}},
            "pages": [
                {"delay_ms": 0_u64, "next_cursor": "second", "items": [{"name": "first"}], "error": null},
                {"delay_ms": 0_u64, "next_cursor": null, "items": [{"name": "second"}], "error": null}
            ]
        }),
    ));
    assert_eq!(
        outcome,
        json!({"items": [{"name": "first"}, {"name": "second"}], "cursors": [null, "second"]})
    );
}

#[test]
fn an_absent_capability_returns_without_fetching() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": null},
            "pages": [{"delay_ms": 0_u64, "next_cursor": null, "items": [{"name": "unused"}], "error": null}]
        }),
    ));
    assert_eq!(outcome, json!({"items": [], "cursors": []}));
}

#[test]
fn a_first_page_method_not_found_returns_an_empty_list() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": {}},
            "pages": [{
                "delay_ms": 0_u64,
                "next_cursor": null,
                "items": [],
                "error": {"code": -32_601_i32, "message": "unsupported"}
            }]
        }),
    ));
    assert_eq!(outcome, json!({"items": [], "cursors": [null]}));
}

#[test]
fn a_repeated_cursor_returns_the_items_collected_so_far() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": {}},
            "pages": [
                {"delay_ms": 0_u64, "next_cursor": "next", "items": [{"name": "first"}], "error": null},
                {"delay_ms": 0_u64, "next_cursor": "next", "items": [{"name": "second"}], "error": null}
            ]
        }),
    ));
    assert_eq!(
        outcome,
        json!({"items": [{"name": "first"}, {"name": "second"}], "cursors": [null, "next"]})
    );
}

#[test]
fn an_empty_cursor_ends_the_walk() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": {}},
            "pages": [
                {"delay_ms": 0_u64, "next_cursor": "", "items": [{"name": "first"}], "error": null},
                {"delay_ms": 0_u64, "next_cursor": null, "items": [{"name": "second"}], "error": null}
            ]
        }),
    ));
    assert_eq!(
        outcome,
        json!({"items": [{"name": "first"}], "cursors": [null]})
    );
}

#[test]
fn a_single_page_within_the_client_timeout_succeeds() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": {}},
            "pages": [{"delay_ms": 45_000_u64, "next_cursor": null, "items": [{"name": "slow"}], "error": null}]
        }),
    ));
    assert_eq!(
        outcome,
        json!({"items": [{"name": "slow"}], "cursors": [null]})
    );
}

#[test]
fn list_items_keep_the_sdk_metadata_fields() {
    let outcome = run(&input(
        "list_resources",
        &json!({
            "capabilities": {"resources": {}},
            "pages": [{
                "delay_ms": 0_u64,
                "next_cursor": null,
                "items": [{
                    "uri": "test://rich",
                    "name": "rich",
                    "title": "Rich resource",
                    "size": 12_i64,
                    "annotations": {"audience": ["user"], "priority": 0.5_f64},
                    "_meta": {"trace": "abc"}
                }],
                "error": null
            }]
        }),
    ));
    assert_eq!(outcome["items"][0]["title"], "Rich resource");
    assert_eq!(outcome["items"][0]["size"], 12_i64);
    assert_eq!(outcome["items"][0]["_meta"]["trace"], "abc");
}

#[test]
fn a_page_past_the_deadline_returns_the_items_collected_so_far() {
    let outcome = run(&input(
        "list_prompts",
        &json!({
            "capabilities": {"prompts": {}},
            "pages": [
                {"delay_ms": 10_000_u64, "next_cursor": "second", "items": [{"name": "first"}], "error": null},
                {"delay_ms": 100_000_u64, "next_cursor": null, "items": [{"name": "second"}], "error": null}
            ]
        }),
    ));
    assert_eq!(
        outcome,
        json!({"items": [{"name": "first"}], "cursors": [null, "second"]})
    );
}

#[test]
fn get_prompt_forwards_the_name_and_arguments() {
    let outcome = run(&input(
        "get_prompt",
        &json!({
            "prompt": {
                "name": "greet",
                "arguments": {"who": "world"},
                "result": {"messages": [{"role": "user", "content": {"type": "text", "text": "hello"}}]},
                "error": null
            }
        }),
    ));
    assert_eq!(
        outcome,
        json!({
            "result": {"messages": [{"role": "user", "content": {"type": "text", "text": "hello"}}]},
            "params": [{"name": "greet", "arguments": {"who": "world"}}]
        })
    );
}

#[test]
fn read_resource_forwards_the_url() {
    let outcome = run(&input(
        "read_resource",
        &json!({
            "resource": {
                "uri": "test://one",
                "result": {"contents": [{"uri": "test://one", "mimeType": "text/plain", "text": "hello"}]},
                "error": null
            }
        }),
    ));
    assert_eq!(
        outcome,
        json!({
            "result": {"contents": [{"uri": "test://one", "mimeType": "text/plain", "text": "hello"}]},
            "uris": ["test://one"]
        })
    );
}

#[test]
fn read_resource_normalizes_the_url() {
    let outcome = run(&input(
        "read_resource",
        &json!({
            "resource": {"uri": "http://example.com", "result": {"contents": []}, "error": null}
        }),
    ));
    assert_eq!(outcome["uris"], json!(["http://example.com/"]));
}

#[test]
fn a_failed_prompt_fetch_reports_the_failure() {
    let value = input(
        "get_prompt",
        &json!({
            "prompt": {
                "name": "greet",
                "arguments": null,
                "result": null,
                "error": {"code": -32_603_i32, "message": "prompt failed"}
            }
        }),
    );
    let registered: Input = serde_json::from_value(value).unwrap();
    let _failure = Op::run(&registered).unwrap_err();
}
