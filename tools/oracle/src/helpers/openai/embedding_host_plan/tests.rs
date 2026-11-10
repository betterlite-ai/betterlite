#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

fn run_case(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/embedding-host-plan")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_embedding_host_plan() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("embedding_host_plan"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert_eq!(seeds.len(), 2);
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "embedding_host_plan"
            && super::Op::decode_and_run(&case.call.input).is_ok()
    }));
}

#[test]
fn mixed_input_keeps_text_and_drops_nonstrings_after_an_image() {
    let result = run_case("jina-mixed-images-and-nonstrings.yaml");
    let expected = json!([
        {"text": "before"}, {"image": "aGVsbG8="}, {"text": "after"},
        {"text": "data:image/png;base64,***"},
    ]);
    assert_eq!(result["jina"]["request"]["input"], expected);
    assert_eq!(
        result["infinity"]["request"]["input"][1],
        "data:image/png;base64,aGVsbG8="
    );
}

#[test]
fn the_normalized_input_beats_the_optional_one_and_the_hook_gets_the_builtin() {
    let result = run_case("jina-input-overlay-and-hook.yaml");
    let request = &result["jina"]["request"];
    assert_eq!(request["input"], json!(["plain"]));
    assert_eq!(request["model"], "other");
    assert_eq!(request["dimensions"], 64_i32);
    assert_eq!(result["jina"]["hook"][0]["input_is_builtin"], true);
    assert_eq!(result["jina"]["hook"][0]["complete_input_dict"], *request);
    assert_eq!(result["infinity"]["request"]["input"], "shadow");
    assert_eq!(result["infinity"]["request"]["output_dimension"], 64_i32);
}

#[test]
fn the_endpoint_suffix_differs_between_the_hosts() {
    let result = run_case("endpoint-suffix-differences.yaml");
    assert_eq!(
        result["jina"]["url"],
        "https://x.test/v1/embeddings/embeddings"
    );
    assert_eq!(result["infinity"]["url"], "https://x.test/v1/embeddings");
}

#[test]
fn aliases_and_caller_headers_resolve_per_host() {
    let result = run_case("aliases-and-header-precedence.yaml");
    assert_eq!(result["jina"]["api_base"], "https://env.test/v1");
    assert_eq!(result["jina"]["api_key"], "second");
    assert_eq!(result["jina"]["url"], "https://api.jina.ai/v1/embeddings");
    assert_eq!(result["jina"]["headers"]["Authorization"], "Bearer mine");
    assert_eq!(result["infinity"]["url"], "/embeddings");
    assert_eq!(
        result["infinity"]["headers"]["Authorization"],
        "Bearer mine"
    );
    assert_eq!(result["infinity"]["headers"]["accept"], "text/plain");
}

#[test]
fn infinity_counters_stay_independent_and_errors_keep_their_headers() {
    let result = run_case("infinity-usage-and-error-headers.yaml");
    let usage = &result["infinity"]["response"]["usage"];
    assert_eq!(
        *usage,
        json!({"prompt_tokens": 5_i32, "total_tokens": 0_i32})
    );
    let error = &result["infinity"]["error"];
    assert_eq!(error["status_code"], json!(429_i32));
    assert_eq!(error["message"], json!("busy"));
    assert_eq!(error["headers"], json!({"retry-after": "3"}));
    assert_eq!(error["url"], "https://github.com/michaelfeil/infinity");
}
