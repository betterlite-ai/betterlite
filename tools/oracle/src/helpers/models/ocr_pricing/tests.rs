#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use serde_json::json;

fn input(value: serde_json::Value) -> Input {
    serde_json::from_value(value).expect("a fixture input parses")
}

#[test]
fn layering_skips_a_string_but_keeps_a_later_zero() {
    let request = input(json!({
        "kind": "layering",
        "sources": [
            {"ocr_cost_per_page": "5"},
            {"ocr_cost_per_page": 0_i64},
        ],
    }));
    let output = Op::run(&request).expect("layering succeeds");
    assert_eq!(output["ocr_cost_per_page"], "0");
}

#[test]
fn sync_cost_bills_pages_from_the_deployment_override() {
    let request = input(json!({
        "kind": "sync",
        "model": "fixture/missing",
        "custom_llm_provider": null,
        "model_info": {"ocr_cost_per_page": 2_i64},
        "usage": {"credits": null, "pages_processed": -5_i64, "pages_processed_annotation": null},
        "catalog": null,
    }));
    let output = Op::run(&request).expect("sync cost succeeds");
    assert_eq!(output["cost"], "-10");
}

#[test]
fn batch_cost_resolves_each_family_independently_from_the_deployment() {
    let request = input(json!({
        "kind": "batch",
        "model": "fixture/missing",
        "custom_llm_provider": null,
        "model_info": {
            "ocr_cost_per_page_batches": 2_i64,
            "annotation_cost_per_page_batches": 3_i64,
        },
        "usage": {"pages_processed": 4_i64, "pages_processed_annotation": 5_i64},
        "catalog": null,
    }));
    let output = Op::run(&request).expect("batch cost succeeds");
    assert_eq!(output["cost"], "23");
}

#[test]
fn batch_cost_falls_back_to_the_injected_catalog_entry_per_family() {
    let request = input(json!({
        "kind": "batch",
        "model": "fixture/missing",
        "custom_llm_provider": null,
        "model_info": {"ocr_cost_per_page": 2_i64},
        "usage": {"pages_processed": 5_i64, "pages_processed_annotation": 3_i64},
        "catalog": {
            "key": "fixture/missing",
            "entry": {"annotation_cost_per_page_batches": 4_i64},
        },
    }));
    let output = Op::run(&request).expect("batch cost succeeds");
    assert_eq!(output["cost"], "22");
}
