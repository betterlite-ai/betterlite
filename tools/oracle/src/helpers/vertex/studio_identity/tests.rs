#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-identity")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_studio_identity() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_identity"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_identity"
    }));
}

#[test]
fn oracle_google_key_precedes_gemini() {
    let result = run_case("google-key-first.yaml");
    let call = &result["calls"][0];
    assert_eq!(call["api_key"], "google-key");
    assert_eq!(call["env_key"], "google-key");
    assert_eq!(
        call["headers"],
        json!({"Content-Type": "application/json", "x-goog-api-key": "google-key"})
    );
}

#[test]
fn oracle_request_key_precedes_the_environment() {
    let result = run_case("request-key-first.yaml");
    let call = &result["calls"][0];
    assert_eq!(call["api_key"], "request-key");
    assert_eq!(call["env_key"], "google-key");
    assert_eq!(call["headers"]["x-goog-api-key"], "request-key");
}

#[test]
fn oracle_native_headers_merge_in_order() {
    let result = run_case("native-header-override.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["headers"],
        json!({
            "Content-Type": "text/plain",
            "X-Caller": "kept",
            "x-goog-api-key": "request-key"
        })
    );
    assert_eq!(
        calls[1]["headers"],
        json!({
            "Content-Type": "application/json",
            "X-Auth": "mapped",
            "x-goog-api-key": "caller-key"
        })
    );
    assert_eq!(
        calls[2]["headers"],
        json!({"Content-Type": "application/json", "x-goog-api-key": "params-key"})
    );
    assert_eq!(calls[3]["headers"]["x-goog-api-key"], "params-gemini-key");
    assert_eq!(calls[4]["headers"]["x-goog-api-key"], "google-key");
    assert_eq!(calls[5]["headers"]["x-goog-api-key"], "generic-key");
    assert_eq!(
        calls[6]["headers"],
        json!({"Content-Type": "application/json"})
    );
}

#[test]
fn oracle_base_defaults_to_the_generative_language_host() {
    let result = run_case("base-default.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["api_base"],
        "https://generativelanguage.googleapis.com"
    );
    assert_eq!(calls[1]["api_base"], "https://proxy.example");
    assert_eq!(calls[2]["api_base"], "https://environment.example");
}

#[test]
fn oracle_repeated_prefixes_are_replaced() {
    let result = run_case("repeated-prefix.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["base_model"], "gemini-2.0-flash");
    assert_eq!(
        calls[0]["model_names"],
        json!(["gemini/gemini-2.0-flash", "gemini/x"])
    );
    assert_eq!(calls[1]["base_model"], "a/b");
    assert_eq!(calls[1]["model_names"], json!(["gemini/gemini-2.0-flash"]));
    assert_eq!(calls[2]["base_model"], "");
    assert_eq!(calls[2]["model_names"], json!([]));
}

#[test]
fn oracle_missing_key_leaves_no_auth_header() {
    let result = run_case("missing-key.yaml");
    let call = &result["calls"][0];
    assert_eq!(call["api_key"], Value::Null);
    assert_eq!(call["env_key"], Value::Null);
    assert_eq!(call["headers"], json!({"Content-Type": "application/json"}));
}
