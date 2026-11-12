#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-interactions")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

fn bodies(result: &Value) -> Vec<&Value> {
    result["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| &call["body"])
        .collect()
}

#[test]
fn helper_seeds_select_studio_interactions() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_interactions"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert_eq!(seeds.len(), 8);
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_interactions"
    }));
}

#[test]
fn oracle_model_wins_over_agent() {
    let result = run_case("model-over-agent.yaml");
    assert_eq!(
        bodies(&result),
        [
            &json!({"model": "gemini-2.5-flash", "input": "hello"}),
            &json!({"agent": "deep-research", "input": "hello"}),
            &json!({"agent": "deep-research"}),
            &json!({"model": "gemini/"}),
        ]
    );
}

#[test]
fn oracle_false_and_empty_values_are_kept() {
    let result = run_case("false-optional-values.yaml");
    let calls = bodies(&result);
    assert_eq!(
        calls[0],
        &json!({
            "model": "gemini-2.5-flash", "input": "", "tools": [], "system_instruction": "",
            "stream": false, "store": false, "background": false, "environment": "",
            "response_modalities": [], "previous_interaction_id": ""
        })
    );
    assert_eq!(calls[1], &json!({"model": "gemini-2.5-flash"}));
}

#[test]
fn oracle_mime_type_wraps_the_legacy_schema() {
    let result = run_case("mime-wrap-schema.yaml");
    let formats: Vec<&Value> = bodies(&result)
        .into_iter()
        .map(|body| &body["response_format"])
        .collect();
    assert_eq!(
        formats,
        [
            &json!({"type": "text", "mime_type": "application/json"}),
            &json!({"type": "text", "mime_type": "application/json", "schema": {"type": "object"}}),
            &json!([]),
            &json!({"mime_type": "text/plain"}),
            &json!("json"),
        ]
    );
}

#[test]
fn oracle_image_config_moves_into_the_format() {
    let result = run_case("image-format-append.yaml");
    let calls = bodies(&result);
    assert_eq!(
        calls[0]["response_format"],
        json!({"type": "image", "aspect_ratio": "1:1"})
    );
    assert_eq!(
        calls[1]["response_format"],
        json!([{"type": "text"}, {"type": "image", "image_size": "2K"}])
    );
    assert_eq!(
        calls[1]["generation_config"],
        json!({"temperature": "warm"})
    );
    assert_eq!(
        calls[2]["response_format"],
        json!([{"type": "text", "mime_type": "application/json"}, {"type": "image"}])
    );
    assert_eq!(calls[3], &json!({"model": "gemini-2.5-flash-image"}));
    assert_eq!(calls[4]["generation_config"], json!({"top_k": "3"}));
    assert!(calls[4].get("response_format").is_none());
}

#[test]
fn oracle_image_type_comes_from_the_config() {
    let result = run_case("image-type-override.yaml");
    assert_eq!(
        bodies(&result)[0]["response_format"],
        json!({"type": "custom", "image_size": "1K"})
    );
}

#[test]
fn oracle_create_keeps_the_projection_and_get_does_not() {
    let result = run_case("create-get-header-difference.yaml");
    let calls = result["calls"].as_array().unwrap();
    let raw = json!({
        "content-length": "62", "x-ratelimit-remaining-requests": "9",
        "x-litellm-spoofed": "yes", "x-request-id": "r1"
    });
    assert_eq!(calls[0]["headers"], raw);
    assert_eq!(calls[1]["headers"], raw);
    assert_eq!(
        calls[0]["additional_headers"],
        json!({
            "x-ratelimit-remaining-requests": "9",
            "llm_provider-content-length": "62",
            "llm_provider-x-ratelimit-remaining-requests": "9",
            "llm_provider-x-litellm-spoofed": "yes",
            "llm_provider-x-request-id": "r1"
        })
    );
    assert_eq!(calls[1]["additional_headers"], Value::Null);
    assert_eq!(calls[0]["record"], calls[1]["record"]);
    assert_eq!(calls[0]["record"]["extra"], json!({"k": 1_i32}));
    assert_eq!(calls[2]["record"]["object"], "interaction");
}

#[test]
fn oracle_lifecycle_ids_are_one_encoded_segment() {
    let result = run_case("encoded-cancel-id.yaml");
    let calls = result["calls"].as_array().unwrap();
    let base = "https://generativelanguage.googleapis.com/v1beta";
    assert_eq!(
        calls[0]["url"],
        format!("{base}/interactions/a%2Fb%20c%3Fd%23e%25:cancel")
    );
    assert_eq!(
        calls[1]["url"],
        "https://proxy.example/v1beta/interactions/a%2Fb%20c%3Fd%23e%25"
    );
    assert_eq!(calls[2]["url"], format!("{base}/interactions/a-b.c_d~e"));
    assert_eq!(calls[3]["url"], format!("{base}/interactions/%C3%A9"));
}

#[test]
fn oracle_delete_succeeds_on_any_2xx_without_reading_the_body() {
    let result = run_case("delete-204.yaml");
    assert_eq!(
        result["calls"],
        json!([
            {"success": true, "id": "v1_a"},
            {"success": true, "id": "v1_b"},
            {"success": true, "id": "v1_c"},
        ])
    );
}
