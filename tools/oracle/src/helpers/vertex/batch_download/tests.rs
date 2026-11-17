#![cfg(test)]

use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

const NOW: i64 = 1_700_000_000;
const URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-flash%2Fbatch.jsonl";
const EMBEDDING_URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-embedding-001%2Fbatch.jsonl";
const ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2,"trafficType":"ON_DEMAND"}}}"#;
const EMBEDDING_ROWS: &str = concat!(
    r#"{"key":"id%23one#1/2","request":{"content":{}},"status":"","response":{"embedding":{"values":[2]},"usageMetadata":{"promptTokenCount":2}}}"#,
    "\n",
    r#"{"key":"id%23one#0/2","request":{"content":{}},"status":"","response":{"embedding":{"values":[1]},"usageMetadata":{"promptTokenCount":1}}}"#,
);

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

fn buffer_input(content: &[u8], url: &str) -> Value {
    json!({
        "operation": "buffer",
        "content": STANDARD.encode(content),
        "url": url,
        "headers": {"Content-Type": "application/json", "Content-Length": "1"},
        "disabled": false,
        "now": NOW,
    })
}

fn content_of(value: &Value) -> Vec<u8> {
    STANDARD.decode(value["content"].as_str().unwrap()).unwrap()
}

fn length(value: &Value) -> usize {
    value["content_length"].as_str().unwrap().parse().unwrap()
}

#[test]
fn helper_seeds_select_vertex_batch_download() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_batch_download"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        validate_helper(Path::new("<helper>"), seed).unwrap();
        seed.call.operation == "vertex_batch_download" && !seed.call.input.is_null()
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({
        "operation": "buffer",
        "content": "",
        "url": "https://x/y",
        "headers": {},
        "disabled": false,
        "now": 0_i64,
        "other": 1_i32,
    }));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn buffer_projects_the_converted_rows() {
    let value = run(buffer_input(ROW.as_bytes(), URL));
    let row = &value["rows"][0];
    assert_eq!(
        row["id"],
        json!("batch_req_00000000-0000-0000-0000-000000000002")
    );
    assert_eq!(row["custom_id"], json!("unknown"));
    assert_eq!(
        row["response"]["body"]["id"],
        json!("chatcmpl-00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(row["response"]["body"]["created"], json!(NOW));
    assert_eq!(
        value["headers"],
        json!({"content-type": "application/json"})
    );
    assert_ne!(value["content_length"], json!("1"));
    assert_eq!(length(&value), content_of(&value).len());
}

#[test]
fn buffer_keeps_bytes_it_cannot_read() {
    let binary: &[u8] = b"\x00\x01\xff\n\x02";
    let value = run(buffer_input(binary, URL));
    assert_eq!(value["rows"], Value::Null);
    assert_eq!(value["content_length"], json!("1"));
    assert_eq!(content_of(&value), binary);
    assert_eq!(
        value["headers"],
        json!({"content-type": "application/json"})
    );
}

#[test]
fn stream_projects_the_converted_row() {
    let split = ROW.len().div_euclid(2);
    let value = run(json!({
        "operation": "stream",
        "chunks": [
            STANDARD.encode(&ROW.as_bytes()[..split]),
            STANDARD.encode(&ROW.as_bytes()[split..]),
        ],
        "url": URL,
        "headers": {"Content-Type": "application/json", "Content-Length": "1"},
        "disabled": false,
        "now": NOW,
    }));
    assert_eq!(value["content_length"], Value::Null);
    assert_eq!(
        value["headers"],
        json!({"content-type": "application/json"})
    );
    assert_eq!(
        value["rows"][0]["response"]["body"]["model"],
        json!("gemini-1.5-flash-001")
    );
}

#[test]
fn stream_regroups_the_embedded_rows() {
    let split = EMBEDDING_ROWS.len().div_euclid(2);
    let value = run(json!({
        "operation": "stream",
        "chunks": [
            STANDARD.encode(&EMBEDDING_ROWS.as_bytes()[..split]),
            STANDARD.encode(&EMBEDDING_ROWS.as_bytes()[split..]),
        ],
        "url": EMBEDDING_URL,
        "headers": {"Content-Length": "1"},
        "disabled": false,
        "now": NOW,
    }));
    assert_eq!(value["rows"][0]["custom_id"], json!("id#one"));
    assert_eq!(
        value["rows"][0]["response"]["body"]["model"],
        json!("gemini-embedding-001")
    );
    assert_ne!(value["content_length"], json!("1"));
    assert_eq!(length(&value), content_of(&value).len());
}
