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
fn helper_seeds_select_vertex_batch_upload() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_batch_upload"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        validate_helper(Path::new("<helper>"), seed).unwrap();
        seed.call.operation == "vertex_batch_upload" && !seed.call.input.is_null()
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({"calls": [], "other": 1_i32}));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn calls_report_the_chunks_in_order() {
    let line = r#"{"custom_id":"a","url":"/v1/embeddings","body":{"input":["x","y"]}}"#;
    let value = run(json!({"calls": [
        {"content": line, "passthrough": false},
        {"content": line, "passthrough": true},
    ]}));
    assert_eq!(
        value[0]["chunks"],
        json!([
            r#"{"key": "a#0/2", "request": {"content": {"parts": [{"text": "x"}]}}}"#,
            "\n{\"key\": \"a#1/2\", \"request\": {\"content\": {\"parts\": [{\"text\": \"y\"}]}}}",
        ])
    );
    assert_eq!(value[1]["chunks"], json!([line]));
}

#[test]
fn a_failing_row_is_a_value_error() {
    let input = serde_json::from_value(
        json!({"calls": [{"content": "{\"url\":\"embeddings\",\"body\":{\"input\":[]}}", "passthrough": false}]}),
    )
    .unwrap();
    let error = super::ops::run(&input).unwrap_err();
    assert_eq!(error.class, "ValueError");
}
