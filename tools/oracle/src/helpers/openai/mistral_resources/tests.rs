#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

fn run_case(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/mistral-resources")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_mistral_resources() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("mistral_resources"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "mistral_resources"
    }));
}

#[test]
fn oracle_ocr_reads_back_as_user_data() {
    let result = run_case("ocr-user-data-round-trip.yaml");
    let files = &result["files"];
    assert_eq!(files["uploads"][0]["purpose"], "batch");
    assert_eq!(files["uploads"][0]["filename"], "scan.pdf");
    assert_eq!(files["uploads"][0]["content_type"], "application/pdf");
    assert_eq!(files["list"][0]["query"], json!([["purpose", "ocr"]]));
    assert_eq!(files["list"][1]["query"], json!([["purpose", "ocr"]]));
    let purposes: Vec<&Value> = files["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|body| &body["purpose"])
        .collect();
    assert_eq!(
        purposes,
        [
            &json!("user_data"),
            &json!("user_data"),
            &json!("user_data"),
            &json!("fine-tune"),
            &json!("batch"),
            &json!("batch"),
        ]
    );
    assert_eq!(files["bodies"][0]["status"], "uploaded");
    assert_eq!(files["bodies"][0]["object"], "file");
    assert_eq!(result["create"], json!([]));
}

#[test]
fn oracle_rejects_purposes_mistral_lacks() {
    let result = run_case("unsupported-purpose-400.yaml");
    let files = &result["files"];
    assert_eq!(files["uploads"][0]["status"], json!(400_i64));
    assert_eq!(files["uploads"][1]["status"], json!(400_i64));
    assert_eq!(files["list"][0]["status"], json!(400_i64));
    assert_eq!(files["list"][1]["status"], json!(400_i64));
    assert_eq!(files["list"][2]["status"], json!(400_i64));
}

#[test]
fn oracle_upload_defaults_and_list_filters() {
    let result = run_case("file-defaults-and-list-purpose.yaml");
    let files = &result["files"];
    assert_eq!(files["uploads"][0]["filename"], "file_1704153600.jsonl");
    assert_eq!(
        files["uploads"][0]["content_type"],
        "application/octet-stream"
    );
    assert_eq!(files["uploads"][0]["content"], json!([1, 2, 3]));
    assert_eq!(files["uploads"][1]["content_type"], "text/plain");
    assert_eq!(files["uploads"][2]["content_type"], "application/x-custom");
    assert_eq!(files["list"][0]["query"], json!([]));
    assert_eq!(files["list"][1]["query"], json!([]));
    assert_eq!(files["list"][2]["query"], json!([["purpose", "batch"]]));
    assert_eq!(files["list"][3]["query"], json!([["purpose", "fine-tune"]]));
}

#[test]
fn oracle_batch_create_keeps_none_apart_from_empty() {
    let result = run_case("batch-none-versus-empty.yaml");
    let calls = result["create"].as_array().unwrap();
    assert_eq!(
        calls[0]["body"],
        json!({
            "input_files": ["file-1"],
            "endpoint": "/v1/chat/completions",
            "model": "mistral-small-latest",
            "metadata": {"name": "team"},
        })
    );
    assert_eq!(calls[0]["url"], "https://api.mistral.ai/v1/batch/jobs");
    assert_eq!(calls[0]["authorization"], "Bearer Key");
    assert_eq!(
        calls[1]["body"],
        json!({"input_files": [""], "endpoint": "/v1/ocr", "model": "mistral-ocr-latest"})
    );
    assert_eq!(calls[1]["url"], "https://proxy.example/v1/batch/jobs");
    assert_eq!(calls[2]["status"], Value::Null);
    assert_eq!(calls[3]["status"], Value::Null);
    assert_eq!(calls[4]["status"], Value::Null);
}

#[test]
fn oracle_retrieve_is_an_explicit_get_with_its_own_key() {
    let result = run_case("native-retrieve-get.yaml");
    let calls = result["retrieve"].as_array().unwrap();
    assert_eq!(calls[0]["method"], "GET");
    assert_eq!(
        calls[0]["url"],
        "https://api.mistral.ai/v1/batch/jobs/job-1"
    );
    assert_eq!(calls[0]["headers"]["Authorization"], "Bearer Key");
    assert_eq!(
        calls[1]["url"],
        "https://proxy.example/v1/batch/jobs/a%20b%2Fc"
    );
    assert_eq!(calls[1]["headers"]["Authorization"], "Bearer EnvKey");
    assert_eq!(calls[2]["status"], Value::Null);
    assert_eq!(
        calls[3]["url"],
        "https://api.mistral.ai/v1/batch/jobs/job-4"
    );
}

#[test]
fn oracle_maps_every_job_status_and_its_terminal_time() {
    let result = run_case("all-job-statuses.yaml");
    let calls = result["jobs"].as_array().unwrap();
    let statuses: Vec<&Value> = calls.iter().map(|call| &call["batch"]["status"]).collect();
    assert_eq!(
        statuses,
        [
            &json!("validating"),
            &json!("in_progress"),
            &json!("completed"),
            &json!("failed"),
            &json!("expired"),
            &json!("cancelling"),
            &json!("cancelled"),
        ]
    );
    assert_eq!(calls[2]["batch"]["completed_at"], json!(200_i64));
    assert_eq!(calls[3]["batch"]["failed_at"], json!(300_i64));
    assert_eq!(calls[4]["batch"]["expired_at"], json!(400_i64));
    assert_eq!(calls[6]["batch"]["cancelled_at"], json!(600_i64));
    assert_eq!(calls[5]["batch"]["completed_at"], Value::Null);
    assert_eq!(calls[0]["batch"]["completion_window"], "24h");
    assert_eq!(calls[1]["batch"]["input_file_id"], "");
}

#[test]
fn oracle_counts_succeeded_requests_and_error_multiplicity() {
    let result = run_case("succeeded-count-and-error-multiplicity.yaml");
    let calls = result["jobs"].as_array().unwrap();
    let counts = &calls[0]["batch"]["request_counts"];
    assert_eq!(counts["completed"], json!(3_i64));
    assert_eq!(counts["failed"], json!(1_i64));
    assert_eq!(counts["total"], json!(9_i64));
    assert_eq!(calls[0]["batch"]["errors"]["data"][0]["message"], "bad row");
    assert_eq!(
        calls[0]["batch"]["errors"]["data"][1]["message"],
        "rate limited (x3)"
    );
    assert_eq!(calls[0]["batch"]["errors"]["data"][2]["message"], "zero");
    assert_eq!(calls[0]["batch"]["output_file_id"], "out-1");
    assert_eq!(calls[0]["batch"]["error_file_id"], "err-1");
    assert_eq!(calls[0]["batch"]["input_file_id"], "");
    assert_eq!(calls[1]["batch"]["input_file_id"], "file-first");
    assert_eq!(calls[0]["batch"]["model"], Value::Null);
}
