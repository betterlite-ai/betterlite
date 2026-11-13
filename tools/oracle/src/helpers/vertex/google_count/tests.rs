#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/google-count")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_google_count() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("google_count"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "google_count"
    }));
}

#[test]
fn oracle_function_response_id_is_removed() {
    let result = run_case("function-response-id.yaml");
    let call = &result["calls"][0];
    assert_eq!(
        call["contents"],
        json!([
            {"role": "user", "parts": [{"text": "Hello world"}]},
            {
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": "read_many_files",
                        "response": {"output": "No files matching the criteria were found."}
                    }
                }]
            }
        ])
    );
    assert_eq!(
        call["original"][1]["parts"][0]["functionResponse"]["id"],
        "read_many_files-1757526647518-730a691aac11c"
    );
    assert_eq!(
        call["url"],
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens"
    );
}

#[test]
fn oracle_none_contents_are_preserved() {
    let result = run_case("none-contents.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["contents"], Value::Null);
    assert_eq!(calls[0]["original"], Value::Null);
    assert_eq!(calls[1]["contents"], json!([]));
    assert_eq!(calls[1]["original"], json!([]));
}

#[test]
fn oracle_original_is_not_mutated() {
    let result = run_case("original-not-mutated.yaml");
    let call = &result["calls"][0];
    assert_eq!(
        call["contents"][0]["parts"][0]["functionResponse"],
        json!({"name": "read_many_files", "response": {"output": "kept"}})
    );
    assert_eq!(
        call["original"][0]["parts"][0]["functionResponse"],
        json!({"id": "call-1", "name": "read_many_files", "response": {"output": "kept"}})
    );
    assert_eq!(
        call["contents"][0]["parts"][1],
        json!({"text": "alongside"})
    );
}

#[test]
fn oracle_studio_base_suffix_is_concatenated() {
    let result = run_case("studio-base-suffix.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["url"],
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens"
    );
    assert_eq!(
        calls[1]["url"],
        "https://proxy.example//v1beta/models/gemini-2.0-flash:countTokens"
    );
    assert_eq!(
        calls[2]["url"],
        "https://proxy.example/openai/v1beta/v1beta/models/gemini-2.0-flash:countTokens"
    );
    assert_eq!(
        calls[3]["url"],
        "https://proxy.example/v1beta/models/gemini-2.5-pro:countTokens"
    );
}
