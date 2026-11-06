#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> serde_json::Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-identity")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_xai_identity() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_identity"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "xai_identity"
    }));
}

#[test]
fn oracle_key_modes_pick_the_documented_source() {
    let result = run_case("key-modes.yaml");
    let calls = result["calls"].as_array().unwrap();
    let chat: Vec<_> = calls.iter().map(|call| call["chat_key"].clone()).collect();
    let legacy: Vec<_> = calls
        .iter()
        .map(|call| call["legacy_key"].clone())
        .collect();
    let null = serde_json::Value::Null;
    let expected = [
        serde_json::Value::from("explicit"),
        serde_json::Value::from("module"),
        serde_json::Value::from("environment"),
        serde_json::Value::from("environment"),
        null.clone(),
        null.clone(),
        null,
    ];
    assert_eq!(chat, expected);
    assert_eq!(legacy[2], "generic");
    assert_eq!(legacy[1], "module");
    assert_eq!(calls[5]["api_base"], "https://proxy.example");
    assert_eq!(calls[4]["api_base"], "https://api.x.ai");
    assert_eq!(calls[6]["api_base"], "https://explicit.example");
}

#[test]
fn oracle_header_casing_keeps_caller_content_type() {
    let result = run_case("header-casing.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["headers"],
        serde_json::json!({"Content-Type": "application/json"})
    );
    assert_eq!(
        calls[3]["headers"],
        serde_json::json!({"Authorization": "Bearer old", "Content-Type": "application/json"})
    );
    assert_eq!(
        calls[5]["headers"],
        serde_json::json!({"content-type": "text/plain"})
    );
    assert_eq!(
        calls[7]["headers"],
        serde_json::json!({"CONTENT-TYPE": "text/plain", "Content-Type": "application/json"})
    );
}

#[test]
fn oracle_provider_prefix_is_removed_everywhere() {
    let result = run_case("repeated-provider-prefix.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["base_model"], "grok-4");
    assert_eq!(calls[1]["base_model"], "grok-4");
    assert_eq!(calls[2]["base_model"], "a/b");
    assert_eq!(
        calls[4]["models"]["ids"],
        serde_json::json!(["xai/grok-4", "xai/xai/grok-3"])
    );
    assert_eq!(calls[9]["models"]["error"], "status");
    assert_eq!(calls[13]["models"]["error"], "malformed");
}
