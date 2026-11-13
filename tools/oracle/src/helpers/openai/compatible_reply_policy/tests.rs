#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/compatible-reply-policy")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_compatible_reply_policy() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("compatible_reply_policy"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "compatible_reply_policy"
    }));
}

#[test]
fn missing_and_empty_headers_and_base_follow_the_python_checks() {
    let validated = &run_case("missing-empty-headers-and-base.yaml")["validate"];
    assert_eq!(validated[0]["status"], 400_i32);
    assert_eq!(validated[1]["status"], 400_i32);
    assert_eq!(validated[2]["headers"], json!({}));
    assert_eq!(validated[3]["headers"]["Authorization"], "Bearer ");
    assert_eq!(validated[4]["status"], 400_i32);
    assert_eq!(validated[5]["url"], "/chat/completions");
    assert_eq!(validated[6]["headers"]["Authorization"], "Bearer k");
    assert_eq!(validated[7]["headers"], json!({"Authorization": "mine"}));
    assert_eq!(validated[8]["url"], "https://api.test/custom");
}

#[test]
fn null_tokens_become_zero_and_only_one_tool_call_converts() {
    let transformed = &run_case("null-token-and-single-tool.yaml")["transform"];
    assert_eq!(transformed[0]["usage"], json!([0_i32, 5_i32, 0_i32]));
    assert_eq!(
        transformed[1]["choices"][0],
        json!({"content": "{\"a\":1}", "tool_calls": null})
    );
    assert_eq!(transformed[2]["choices"][0]["content"], "{\"b\":2}");
    assert_eq!(transformed[3]["choices"][0]["content"], "");
    assert_eq!(
        transformed[4]["choices"][0],
        json!({"content": null, "tool_calls": 2_i32})
    );
    assert_eq!(
        transformed[5]["choices"][0],
        json!({"content": null, "tool_calls": 1_i32})
    );
    assert_eq!(transformed[6]["usage"], json!([0_i32, 5_i32, 0_i32]));
}

#[test]
fn an_empty_provider_still_prefixes_and_an_empty_base_model_is_hidden() {
    let transformed = &run_case("provider-prefix-and-hidden-model.yaml")["transform"];
    assert_eq!(transformed[0]["model"], "acme/m");
    assert_eq!(transformed[1]["model"], "/m");
    assert_eq!(transformed[1]["hidden_model"], "base");
    assert_eq!(transformed[2]["hidden_model"], "");
    assert_eq!(transformed[3]["hidden_model"], Value::Null);
}

#[test]
fn embedding_options_keep_false_and_zero_and_override_model_and_input() {
    let options = &run_case("embedding-option-filter.yaml")["options"];
    assert_eq!(
        options[0],
        json!({"model": "m", "input": ["a"], "dimensions": 0_i32, "flag": false, "tag": "x", "empty": []})
    );
    assert_eq!(options[1], json!({"model": "other", "input": ["b"]}));
    assert_eq!(options[2], json!({"model": "m", "input": ["a"]}));
}
