#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

fn case_input(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/sambanova-wire")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    case.call.input
}

fn run_case(file: &str) -> Value {
    super::Op::decode_and_run(&case_input(file)).unwrap()
}

#[test]
fn helper_seeds_select_sambanova_wire() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("sambanova_wire-"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert_eq!(seeds.len(), 3);
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "b6413"
            && super::Op::decode_and_run(&case.call.input).is_ok()
    }));
}

#[test]
fn the_chat_case_reports_the_pinned_supported_params_and_mapping() {
    let result = run_case("chat-params-and-messages.yaml");
    assert_eq!(
        result["chat_supported"][0]["supported"],
        json!([
            "max_completion_tokens",
            "max_tokens",
            "response_format",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "top_k",
        ])
    );
    assert_eq!(
        result["chat_params"][0]["mapped"],
        json!({"max_tokens": 20_i32, "temperature": 0.2_f64})
    );
    assert_eq!(
        result["chat_params"][2]["mapped"],
        json!({
            "response_format": {"type": "json_object"},
            "tools": [{"type": "function", "function": {"name": "f"}}],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
        })
    );
    assert_eq!(
        result["chat_params"][3]["mapped"],
        json!({"response_format": {"type": "json_object"}})
    );
    assert_eq!(
        result["chat_params"][4]["mapped"],
        json!({"max_tokens": null, "temperature": null})
    );
}

#[test]
fn the_embedding_case_reports_the_urls() {
    let result = run_case("embedding-url-headers-body.yaml");
    assert_eq!(
        result["embedding_urls"][0]["url"],
        json!("https://api.sambanova.ai/v1/embeddings")
    );
    assert_eq!(result["embedding_urls"][5]["url"], Value::Null);
}

#[test]
fn the_embedding_case_reports_the_headers() {
    let result = run_case("embedding-url-headers-body.yaml");
    assert_eq!(
        result["embedding_headers"][1]["headers"]["Authorization"],
        json!("Bearer None")
    );
    assert_eq!(
        result["embedding_headers"][2]["headers"]["Authorization"],
        json!("Basic abc")
    );
    assert_eq!(
        result["embedding_headers"][3]["headers"]["authorization"],
        json!("Basic abc")
    );
    assert_eq!(
        result["embedding_headers"][5]["headers"]["Authorization"],
        json!("Bearer sk-explicit")
    );
    assert_eq!(
        result["embedding_headers"][6]["headers"]["Authorization"],
        json!("Bearer sk-env")
    );
    assert_eq!(
        result["embedding_headers"][7]["headers"]["Authorization"],
        json!("Bearer ")
    );
}

#[test]
fn the_embedding_case_reports_the_request_bodies() {
    let result = run_case("embedding-url-headers-body.yaml");
    assert_eq!(
        result["embedding_requests"][2]["request"],
        json!({"input": "text", "model": "other"})
    );
}

#[test]
fn the_embedding_case_reports_the_replies() {
    let result = run_case("embedding-url-headers-body.yaml");
    assert_eq!(
        result["embedding_replies"][0]["model"],
        json!("embed-model")
    );
    assert_eq!(
        result["embedding_replies"][0]["usage"],
        json!({"prompt_tokens": 3_i32, "total_tokens": 3_i32})
    );
    assert_eq!(result["embedding_replies"][1]["model"], Value::Null);
    assert_eq!(
        result["embedding_replies"][1]["usage"],
        json!({"prompt_tokens": 0_i32, "total_tokens": 0_i32})
    );
    assert_eq!(result["embedding_replies"][2]["data"], Value::Null);
    assert_eq!(
        result["embedding_replies"][3]["usage"],
        json!({"prompt_tokens": 4_i32, "total_tokens": 0_i32})
    );
    assert_eq!(
        result["embedding_replies"][4]["usage"],
        json!({"prompt_tokens": 0_i32, "total_tokens": 9_i32})
    );
}

#[test]
fn the_embedding_case_reports_the_errors() {
    let result = run_case("embedding-url-headers-body.yaml");
    assert_eq!(
        result["embedding_errors"][0]["request_url"],
        json!("https://docs.litellm.ai/docs")
    );
    assert_eq!(
        result["embedding_errors"][0]["headers"],
        json!({"retry-after": "3"})
    );
    assert_eq!(
        result["embedding_errors"][0]["response_status"],
        json!(429_i32)
    );
    assert_eq!(
        result["embedding_errors"][1]["response_status"],
        json!(400_i32)
    );
}

#[test]
fn the_reply_case_reports_the_pinned_failure_status() {
    let error = super::Op::decode_and_run(&case_input("embedding-reply.yaml")).unwrap_err();
    assert_eq!(error.class, "SambaNovaError");
    assert_eq!(error.status, Some(502));
    assert_eq!(error.message, "upstream unavailable");
}
