#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

fn run_case(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/sambanova-wire")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
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
fn the_chat_case_flattens_message_content_like_litellm() {
    let result = run_case("chat-params-and-messages.yaml");
    assert_eq!(
        result["chat_messages"][0]["messages"][0]["content"],
        json!("one two")
    );
    assert_eq!(
        result["chat_messages"][0]["messages"][1]["content"],
        json!("plainsunny")
    );
    assert_eq!(
        result["chat_messages"][0]["messages"][2]["content"],
        json!([{"type": "image_url", "image_url": {"url": "http://x.test/a.png"}}])
    );
    assert_eq!(
        result["chat_messages"][0]["messages"][3],
        json!({"role": "assistant"})
    );
    assert_eq!(
        result["chat_messages"][0]["messages"][4]["tool_calls"][0]["function"]["name"],
        json!("f")
    );
}

#[test]
fn the_embedding_case_reports_urls_headers_and_bodies() {
    let result = run_case("embedding-url-headers-body.yaml");
    assert_eq!(
        result["embedding_urls"][0]["url"],
        json!("https://api.sambanova.ai/v1/embeddings")
    );
    assert_eq!(result["embedding_urls"][5]["url"], Value::Null);
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
    assert_eq!(
        result["embedding_requests"][2]["request"],
        json!({"input": "text", "model": "other"})
    );
}

#[test]
fn the_reply_case_decodes_successes_and_keeps_failures_bare() {
    let result = run_case("embedding-reply.yaml");
    assert_eq!(
        result["embedding_replies"][0]["reply"]["model"],
        json!("embed-model")
    );
    assert_eq!(
        result["embedding_replies"][0]["reply"]["usage"],
        json!({"prompt_tokens": 3_i32, "total_tokens": 3_i32})
    );
    assert_eq!(result["embedding_replies"][2]["reply"]["data"], Value::Null);
    assert_eq!(result["embedding_replies"][5]["status"], json!(502_i32));
    assert_eq!(result["embedding_replies"][6]["status"], Value::Null);
    assert_eq!(
        result["embedding_errors"][0]["request_url"],
        json!("https://docs.litellm.ai/docs")
    );
}
