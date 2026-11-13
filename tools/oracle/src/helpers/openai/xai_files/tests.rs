#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> serde_json::Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-files")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_xai_files() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_files"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok() && case.call.operation == "xai_files"
    }));
}

#[test]
fn oracle_upload_normalizes_the_returned_file() {
    let result = run_case("upload-defaults.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["supported"], serde_json::json!(["purpose"]));
    assert_eq!(
        calls[0]["mapped"],
        serde_json::json!({"purpose": "batch", "expires_after": {"anchor": "created_at"}})
    );
    assert_eq!(calls[0]["url"], "https://api.x.ai/v1/files");
    assert_eq!(calls[0]["authorization"], "Bearer sk-key");
    assert_eq!(
        calls[0]["file"],
        serde_json::json!({
            "id": "file-1",
            "bytes": 0_i64,
            "created_at": 1_700_000_000_i64,
            "filename": "",
            "object": "file",
            "purpose": "batch",
            "status": "uploaded",
        })
    );
    assert_eq!(calls[1]["url"], "https://proxy.example/v1/files");
    assert_eq!(calls[1]["file"]["purpose"], "batch");
    assert_eq!(calls[1]["file"]["status"], "uploaded");
    assert_eq!(calls[1]["file"]["created_at"], 12_i64);
    assert_eq!(calls[1]["file"]["expires_at"], 5_i64);
    assert_eq!(calls[2]["file"], serde_json::json!({"raised": "reply"}));
}

#[test]
fn oracle_forced_purpose_and_the_fixed_clock() {
    let result = run_case("forced-purpose-and-clock.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["url"], "https://env.example/v1/files");
    assert_eq!(calls[0]["authorization"], "Bearer module-key");
    assert_eq!(calls[0]["file"]["purpose"], "batch");
    assert_eq!(calls[0]["file"]["created_at"], 11_i64);
    assert_eq!(calls[1]["authorization"], "Bearer explicit-key");
    assert_eq!(calls[1]["file"]["created_at"], 1_700_000_900_i64);
}

#[test]
fn oracle_file_ids_are_encoded_and_a_missing_one_fails() {
    let result = run_case("encoded-id-and-missing-content-id.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["delete"]["url"],
        "https://api.x.ai/v1/files/a%2Fb%20c"
    );
    assert_eq!(
        calls[0]["content"]["url"],
        "https://api.x.ai/v1/files/a%2Fb%20c/content"
    );
    assert_eq!(
        calls[0]["deleted"],
        serde_json::json!({"id": "file-1", "object": "file", "deleted": true})
    );
    assert_eq!(
        calls[0]["content_reply"],
        serde_json::json!({"status": 500_i64, "headers": {"x-a": "b"}, "body": [114_i64, 97_i64, 119_i64]})
    );
    assert_eq!(calls[1]["delete"], serde_json::json!({"raised": "path"}));
    assert_eq!(calls[1]["content"], serde_json::json!({"raised": "path"}));
    assert_eq!(calls[1]["deleted"]["deleted"], false);
    assert_eq!(calls[1]["content_reply"]["status"], 206_i64);
    assert_eq!(calls[2]["delete"], serde_json::json!({"raised": "path"}));
    assert_eq!(calls[2]["content"], serde_json::json!({"raised": "path"}));
    assert_eq!(calls[2]["deleted"], serde_json::json!({"raised": "reply"}));
}

#[test]
fn oracle_list_stops_without_a_token_and_a_file() {
    let result = run_case("list-stop-and-delete-default.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["url"], "https://api.x.ai/v1/files");
    assert_eq!(
        calls[0]["next"],
        serde_json::json!({
            "url": "https://api.x.ai/v1/files",
            "query": [["pagination_token", "tok"]],
        })
    );
    assert_eq!(calls[0]["files"][0]["id"], "f1");
    assert_eq!(calls[0]["files"][0]["created_at"], 1_700_000_000_i64);
    assert_eq!(calls[0]["files"][0]["purpose"], "batch");
    assert_eq!(calls[1]["url"], "https://p.example/v1/files");
    assert_eq!(calls[1]["next"], serde_json::Value::Null);
    assert_eq!(calls[2]["next"], serde_json::Value::Null);
    assert_eq!(calls[2]["files"][0]["created_at"], 1_700_000_200_i64);
    assert_eq!(calls[3]["raised"], "page");
    assert_eq!(calls[3]["raised_list"], "list");
    assert_eq!(calls[3]["next"], serde_json::Value::Null);
}
