#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

#[test]
fn helper_seeds_select_vertex_fine_tuning() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_fine_tuning"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "vertex_fine_tuning"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn a_request_call_returns_the_vertex_job() {
    let value = run(json!({
        "operation": "request",
        "calls": [{
            "data": {
                "model": "gemini-1.5-flash-002",
                "training_file": "gs://bucket/train.jsonl",
                "suffix": "my-run",
                "validation_file": "gs://bucket/valid.jsonl",
                "hyperparameters": {"n_epochs": 3_i64, "learning_rate_multiplier": 0.1_f64},
            },
            "original_hyperparameters": {"adapter_size": "ADAPTER_SIZE_ONE"},
        }],
    }));
    let job = &value["calls"][0]["job"];
    assert_eq!(job["baseModel"], json!("gemini-1.5-flash-002"));
    assert_eq!(job["tunedModelDisplayName"], json!("my-run"));
    assert_eq!(
        job["supervisedTuningSpec"]["training_dataset_uri"],
        json!("gs://bucket/train.jsonl")
    );
    assert_eq!(
        job["supervisedTuningSpec"]["validation_dataset"],
        json!("gs://bucket/valid.jsonl")
    );
    assert_eq!(
        job["supervisedTuningSpec"]["hyperParameters"]["epoch_count"],
        json!(3_i64)
    );
    assert_eq!(
        job["supervisedTuningSpec"]["hyperParameters"]["adapter_size"],
        json!("ADAPTER_SIZE_ONE")
    );
}

#[test]
fn a_zero_hyperparameter_call_omits_the_object() {
    let value = run(json!({
        "operation": "request",
        "calls": [{
            "data": {
                "model": "m",
                "training_file": "gs://bucket/train.jsonl",
                "hyperparameters": {"n_epochs": 0_i64, "learning_rate_multiplier": 0.0_f64, "batch_size": 4_i64},
            },
            "original_hyperparameters": {"adapter_size": ""},
        }],
    }));
    assert_eq!(
        value["calls"][0]["job"]["supervisedTuningSpec"],
        json!({"training_dataset_uri": "gs://bucket/train.jsonl"})
    );
}

#[test]
fn a_response_call_maps_the_openai_job() {
    let value = run(json!({
        "operation": "response",
        "calls": [{
            "zone": "UTC",
            "response": {
                "name": "projects/p/locations/us-central1/tuningJobs/9",
                "baseModel": "gemini-1.5-flash-002",
                "supervisedTuningSpec": {"trainingDatasetUri": "gs://bucket/train.jsonl"},
                "state": "JOB_STATE_PARTIALLY_SUCCEEDED",
                "createTime": "2024-01-01T00:00:00Z",
            },
        }],
    }));
    let job = &value["calls"][0]["job"];
    assert_eq!(
        job["id"],
        json!("projects/p/locations/us-central1/tuningJobs/9")
    );
    assert_eq!(job["status"], json!("queued"));
    assert_eq!(job["created_at"], json!(1_704_067_200_i64));
    assert_eq!(job["training_file"], json!("gs://bucket/train.jsonl"));
}

#[test]
fn a_response_without_a_state_reports_no_job() {
    let value = run(json!({
        "operation": "response",
        "calls": [{"zone": "UTC", "response": {"name": "n"}}],
    }));
    assert_eq!(value["calls"][0]["ok"], json!(false));
    assert_eq!(value["calls"][0].get("job"), None);
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(
        json!({"operation": "request", "other": 1_i32}),
    );
    assert!(unknown.unwrap_err().to_string().contains("other"));
}
