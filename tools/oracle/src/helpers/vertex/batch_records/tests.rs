#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;

fn input(value: Value) -> super::ops::Input {
    serde_json::from_value(value).unwrap()
}

fn run(value: Value) -> Value {
    super::ops::run(&input(value)).unwrap()
}

#[test]
fn helper_seeds_select_vertex_batch_records() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_batch_records"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "vertex_batch_records"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown =
        serde_json::from_value::<super::ops::Input>(json!({"operation": "row", "other": 1_i32}));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn a_create_call_returns_the_production_batch_job() {
    let value = run(json!({
        "operation": "create",
        "calls": [{
            "data": {"input_file_id": "gs://bucket/publishers/google/models/gemini-pro/file.jsonl"},
            "vertex_project": null,
            "vertex_location": null,
        }],
    }));
    let calls = value["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["ok"], json!(true));
    assert_eq!(
        calls[0]["job"]["model"],
        json!("publishers/google/models/gemini-pro")
    );
    assert_eq!(
        calls[0]["job"]["inputConfig"]["gcsSource"]["uris"],
        json!(["gs://bucket/publishers/google/models/gemini-pro/file.jsonl"])
    );
    assert_eq!(
        calls[0]["job"]["outputConfig"]["gcsDestination"]["outputUriPrefix"],
        json!("gs://bucket/publishers/google/models/gemini-pro")
    );
}

#[test]
fn a_missing_input_file_reports_no_status() {
    let value = run(json!({
        "operation": "create",
        "calls": [{
            "data": {},
            "vertex_project": "proj",
            "vertex_location": null,
        }],
    }));
    assert_eq!(value["calls"][0]["ok"], json!(false));
    assert_eq!(value["calls"][0]["status"], Value::Null);
}

#[test]
fn a_job_call_projects_its_openai_fields() {
    let value = run(json!({
        "operation": "job",
        "zone": "UTC",
        "calls": [{"response": {
            "name": "projects/p/locations/l/batchPredictionJobs/42",
            "createTime": "2024-01-01T00:00:00.000000Z",
            "state": "JOB_STATE_PARTIALLY_SUCCEEDED",
            "inputConfig": {"gcsSource": {"uris": ["gs://b/in.jsonl"]}},
            "outputInfo": {"gcsOutputDirectory": "gs://b/out"},
        }}],
    }));
    let batch = &value["calls"][0]["batch"];
    assert_eq!(batch["id"], json!("42"));
    assert_eq!(batch["status"], json!("completed"));
    assert_eq!(batch["created_at"], json!(1_704_067_200_i64));
    assert_eq!(batch["input_file_id"], json!("gs://b/in.jsonl"));
    assert_eq!(batch["output_file_id"], json!("gs://b/out/predictions.jsonl"));
    assert_eq!(batch["error_file_id"], Value::Null);
}

#[test]
fn a_malformed_job_reports_no_status() {
    let value = run(json!({
        "operation": "job",
        "zone": "UTC",
        "calls": [{"response": {"name": "n", "state": "JOB_STATE_WHATEVER"}}],
    }));
    assert_eq!(value["calls"][0]["ok"], json!(false));
    assert_eq!(value["calls"][0]["status"], Value::Null);
}

#[test]
fn a_list_call_projects_its_page() {
    let value = run(json!({
        "operation": "list",
        "zone": "UTC",
        "calls": [{"page": {"batchPredictionJobs": [], "nextPageToken": "cursor"}}],
    }));
    let page = &value["calls"][0]["page"];
    assert_eq!(page["object"], json!("list"));
    assert_eq!(page["data"], json!([]));
    assert_eq!(page["first_id"], Value::Null);
    assert_eq!(page["last_id"], Value::Null);
    assert_eq!(page["has_more"], json!(true));
    assert_eq!(page["next_page_token"], json!("cursor"));
}

#[test]
fn a_row_call_prices_a_native_embedding_row() {
    let value = run(json!({
        "operation": "row",
        "calls": [{
            "row": {"request": {}, "response": {"embedding": {"values": [1_i32]}, "tokenCount": 12_i32}},
            "model_name": null,
            "model_info": null,
            "pricing": "fixed",
        }],
    }));
    let stats = &value["calls"][0]["stats"];
    assert_eq!(stats["usage"]["prompt_tokens"], json!(12_i64));
    assert_eq!(stats["usage"]["total_tokens"], json!(12_i64));
    assert_eq!(stats["total_tokens"], json!(12_i64));
    assert_eq!(stats["model"], Value::Null);
    assert_eq!(stats["prompt_cost"], json!("0"));
    assert_eq!(stats["completion_cost"], json!("0"));
}

#[test]
fn a_row_call_without_a_usable_response_returns_null_stats() {
    let value = run(json!({
        "operation": "row",
        "calls": [{
            "row": {"request": {}},
            "model_name": "deployment",
            "model_info": null,
            "pricing": "fixed",
        }],
    }));
    assert_eq!(value["calls"][0]["ok"], json!(true));
    assert_eq!(value["calls"][0]["stats"], Value::Null);
}
