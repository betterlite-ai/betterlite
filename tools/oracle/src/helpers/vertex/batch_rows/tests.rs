#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

#[test]
fn helper_seeds_select_vertex_batch_rows() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_batch_rows"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        validate_helper(Path::new("<helper>"), seed).unwrap();
        seed.call.operation == "vertex_batch_rows" && !seed.call.input.is_null()
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown =
        serde_json::from_value::<super::ops::Input>(json!({"operation": "encode", "other": 1_i32}));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn encode_reports_the_lossy_and_raw_labels() {
    let value = run(json!({"operation": "encode", "custom_ids": ["H\u{e9}llo.req-1"]}));
    let row = &value["rows"][0];
    assert_eq!(row["custom_id"], json!("H\u{e9}llo.req-1"));
    assert_eq!(row["labels"]["litellm_custom_id"], json!("h_llo_req-1"));
    assert!(
        row["labels"]["litellm_custom_id_raw"]
            .as_str()
            .is_some_and(|raw| raw.starts_with("b32_"))
    );
}

#[test]
fn rows_report_the_identifier_and_the_row_kind() {
    let value = run(json!({
        "operation": "rows",
        "rows": [
            {"request": {}, "response": {"embedding": {"values": [1_i32]}}},
            {"request": {}, "response": {"candidates": []}, "processed_time": ""},
            {"request": {"labels": {"litellm_custom_id_raw": "b32_zz"}}},
        ],
    }));
    assert_eq!(value["rows"][0]["custom_id"], json!("unknown"));
    assert_eq!(value["rows"][0]["embeddings"], json!(true));
    assert_eq!(value["rows"][0]["generate"], json!(false));
    assert_eq!(value["rows"][1]["generate"], json!(true));
    assert_eq!(value["rows"][2]["custom_id"], json!("b32_zz"));
}

#[test]
fn rows_report_the_fan_out_suffix() {
    let value = run(json!({"operation": "rows", "rows": [{"key": "a%23b#1/2"}]}));
    assert_eq!(value["rows"][0]["custom_id"], json!("a#b"));
    assert_eq!(value["rows"][0]["index"], json!(1_u32));
    assert_eq!(value["rows"][0]["total"], json!(2_u32));
}

#[test]
fn regroup_builds_one_output_row_per_identifier() {
    let value = run(json!({
        "operation": "regroup",
        "rows": [
            {"key": "id#1/2", "request": {}, "status": "", "response": {"embedding": {"values": [2_i32]}, "tokenCount": 1_i64}},
            {"key": "id#0/2", "request": {}, "status": "", "response": {"embedding": {"values": [1_i32]}, "tokenCount": 1_i64}},
        ],
        "model": "text-embedding-005",
    }));
    let row = &value["rows"][0];
    assert_eq!(
        row["id"],
        json!("batch_req_00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(row["custom_id"], json!("id"));
    assert_eq!(row["response"]["status_code"], json!(200_u16));
    assert_eq!(
        row["response"]["body"]["model"],
        json!("text-embedding-005")
    );
    assert_eq!(
        row["response"]["body"]["usage"]["total_tokens"],
        json!(2_u32)
    );
}

#[test]
fn urls_report_passthrough_and_model() {
    let value = run(json!({
        "operation": "urls",
        "urls": [
            "gs://b/litellm-vertex-files/passthrough/x",
            "https://x/publishers/google/models/gemini-1.5/?a=1",
        ],
    }));
    assert_eq!(
        value["rows"][0],
        json!({"passthrough": true, "model": null})
    );
    assert_eq!(
        value["rows"][1],
        json!({"passthrough": false, "model": "gemini-1.5"})
    );
}
