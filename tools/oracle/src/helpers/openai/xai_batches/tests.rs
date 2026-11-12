#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> serde_json::Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-batches")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_xai_batches() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_batches"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "xai_batches"
    }));
}

#[test]
fn oracle_create_keeps_the_name_and_the_caller_endpoint() {
    let result = run_case("create-name-and-missing-input.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["body"],
        serde_json::json!({"name": "team-batch", "input_file_id": "file-1"})
    );
    assert_eq!(calls[0]["url"], "https://api.x.ai/v1/batches");
    assert_eq!(calls[0]["authorization"], "Bearer Key");
    assert_eq!(calls[0]["batch"]["endpoint"], "/v1/embeddings");
    assert_eq!(calls[0]["batch"]["status"], "completed");
    assert_eq!(calls[0]["batch"]["output_file_id"], "batch_1");
    assert_eq!(
        calls[1]["body"],
        serde_json::json!({"name": "litellm-batch", "input_file_id": "file-2"})
    );
    assert_eq!(calls[1]["url"], "https://proxy.example/v1/batches");
    assert_eq!(calls[1]["batch"]["endpoint"], "/v1/chat/completions");
    assert_eq!(calls[2]["status"], serde_json::json!(400_i64));
    assert_eq!(calls[3]["status"], serde_json::json!(401_i64));
}

#[test]
fn oracle_cancellation_wins_over_the_counters() {
    let result = run_case("cancel-over-counters.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["batch"]["status"], "failed");
    assert_eq!(
        calls[0]["batch"]["failed_at"],
        serde_json::json!(1_704_153_600_i64)
    );
    assert_eq!(calls[1]["batch"]["status"], "cancelled");
    assert_eq!(
        calls[1]["batch"]["cancelled_at"],
        serde_json::json!(1_704_153_600_i64)
    );
    assert_eq!(calls[2]["batch"]["status"], "validating");
    assert_eq!(calls[3]["batch"]["status"], "in_progress");
    assert_eq!(calls[4]["batch"]["status"], "completed");
    assert_eq!(
        calls[4]["batch"]["request_counts"]["failed"],
        serde_json::json!(3_i64)
    );
}

#[test]
fn oracle_timestamps_are_unix_seconds() {
    let result = run_case("date-only-and-invalid-time.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["batch"]["created_at"],
        serde_json::json!(1_704_153_600_i64)
    );
    assert_eq!(
        calls[0]["batch"]["expires_at"],
        serde_json::json!(1_735_603_200_i64)
    );
    assert_eq!(
        calls[1]["batch"]["created_at"],
        serde_json::json!(1_704_164_645_i64)
    );
    assert_eq!(
        calls[1]["batch"]["expires_at"],
        serde_json::json!(1_704_157_445_i64)
    );
    assert_eq!(
        calls[2]["batch"]["created_at"],
        serde_json::json!(1_704_164_645_i64)
    );
    assert_eq!(calls[3]["batch"]["created_at"], serde_json::json!(0_i64));
    assert_eq!(calls[3]["batch"]["expires_at"], serde_json::json!(null));
    assert_eq!(calls[4]["batch"]["created_at"], serde_json::json!(0_i64));
    assert_eq!(
        calls[5]["batch"]["cancelled_at"],
        serde_json::json!(1_704_153_600_i64)
    );
}

#[test]
fn oracle_lists_report_the_token_apart_from_the_rows() {
    let result = run_case("empty-list-with-token.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["page"]["has_more"], serde_json::json!(true));
    assert_eq!(calls[0]["page"]["next_page_token"], "next-token");
    assert_eq!(calls[0]["page"]["first_id"], serde_json::json!(null));
    assert_eq!(calls[1]["page"]["first_id"], "batch_a");
    assert_eq!(calls[1]["page"]["last_id"], "batch_b");
    assert_eq!(calls[1]["page"]["has_more"], serde_json::json!(false));
    assert_eq!(calls[3]["page"]["next_page_token"], serde_json::json!(null));
}

#[test]
fn oracle_jsonl_unwraps_the_first_map_value() {
    let result = run_case("first-response-map-value.yaml");
    let text = result["calls"][0]["jsonl"].as_str().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"request_id\": \"chatcmpl-1\""));
    assert!(lines[0].contains("\"object\": \"chat.completion\""));
    assert!(!lines[0].contains("ignored"));
    assert!(lines[1].contains("\"request_id\": \"resp-1\""));
    assert!(lines[2].contains("\"code\": \"5\""));
    assert!(lines[2].contains("\"message\": \"empty map\""));
}

#[test]
fn oracle_params_default_the_limit_and_keep_an_empty_cursor() {
    let result = run_case("empty-cursor-zero-limit.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["url"],
        "https://api.x.ai/v1/batches?limit=1000&pagination_token="
    );
    assert_eq!(calls[1]["url"], "https://api.x.ai/v1/batches?limit=1000");
    assert_eq!(
        calls[2]["url"],
        "https://proxy.example/v1/batches?limit=5&pagination_token=tok-1"
    );
    assert_eq!(calls[3]["url"], "https://api.x.ai/v1/batches?limit=-1");
    assert_eq!(calls[4]["status"], serde_json::json!(401_i64));
}
