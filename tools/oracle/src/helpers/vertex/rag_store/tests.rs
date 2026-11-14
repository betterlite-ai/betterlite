#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

#[test]
fn helper_seeds_select_vertex_rag_store() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_rag_store"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        seed.call.operation == "vertex_rag_store"
            && seed
                .call
                .input
                .as_object()
                .is_some_and(|input| !input.is_empty())
    }));
}

#[test]
fn a_search_call_maps_the_options() {
    let value = run(json!({
        "calls": [{
            "function": "search",
            "vector_store_id": "projects/p/locations/us-central1/ragCorpora/c",
            "query": ["one", "two"],
            "api_base": "https://host/v1",
            "litellm_params": {},
            "optional_params": {"max_num_results": 0},
        }],
    }));
    let call = &value["calls"][0];
    assert_eq!(call["url"], json!("https://host/v1:retrieveContexts"));
    assert_eq!(call["body"]["query"]["text"], json!("one two"));
    assert_eq!(
        call["body"]["query"]["rag_retrieval_config"],
        json!({"top_k": 0})
    );
    assert_eq!(
        call["body"]["vertex_rag_store"]["rag_resources"][0]["rag_corpus"],
        json!("projects/p/locations/us-central1/ragCorpora/c")
    );
}

#[test]
fn a_bare_corpus_id_expands_under_the_configured_settings() {
    let value = run(json!({
        "calls": [{
            "function": "search",
            "vector_store_id": "corpus",
            "query": "hello",
            "api_base": "https://host",
            "litellm_params": {"vertex_project": "p", "vertex_location": "us-central1"},
            "optional_params": {},
        }],
    }));
    let call = &value["calls"][0];
    assert_eq!(call["url"], json!("https://host:retrieveContexts"));
    assert_eq!(
        call["body"]["vertex_rag_store"]["rag_resources"][0]["rag_corpus"],
        json!("projects/p/locations/us-central1/ragCorpora/corpus")
    );
    assert_eq!(call["body"]["query"], json!({"text": "hello"}));
    assert_eq!(call.get("results"), None);
}

#[test]
fn a_search_response_returns_the_decoded_page() {
    let value = run(json!({
        "calls": [{
            "function": "search",
            "vector_store_id": "projects/p/locations/l/ragCorpora/c",
            "query": "hello",
            "api_base": "https://host",
            "litellm_params": {},
            "optional_params": {},
            "response": {
                "status": 200,
                "body": r#"{"contexts":{"contexts":[{"text":"chunk","sourceDisplayName":"doc"}]}}"#,
            },
        }],
    }));
    let results = &value["calls"][0]["results"];
    assert_eq!(results["search_query"], json!("hello"));
    assert_eq!(results["data"][0]["file_id"], json!("doc"));
    assert_eq!(results["data"][0]["filename"], json!("doc"));
    assert_eq!(results["data"][0]["score"], json!(0.0));
    assert_eq!(
        results["data"][0]["content"],
        json!([{"text": "chunk", "type": "text"}])
    );
}

#[test]
fn a_create_request_takes_the_default_corpus_name() {
    let value = run(json!({
        "calls": [{
            "function": "create_request",
            "api_base": "https://host/v1/projects/p/locations/l",
            "optional_params": {},
        }],
    }));
    let call = &value["calls"][0];
    assert_eq!(
        call["url"],
        json!("https://host/v1/projects/p/locations/l/ragCorpora")
    );
    assert_eq!(
        call["body"],
        json!({
            "display_name": "litellm-vector-store",
            "description": "Vector store created via LiteLLM",
        })
    );
}

#[test]
fn a_create_response_maps_the_corpus() {
    let value = run(json!({
        "calls": [{
            "function": "create_response",
            "status": 200,
            "body": r#"{"name":"projects/p/locations/l/ragCorpora/corpus","display_name":"store","createTime":"2024-01-01T00:00:00Z"}"#,
        }],
    }));
    let created = &value["calls"][0]["created"];
    assert_eq!(created["id"], json!("corpus"));
    assert_eq!(created["name"], json!("store"));
    assert_eq!(created["created_at"], json!(1_704_067_200));
    assert_eq!(created["status"], json!("completed"));
    assert_eq!(created["metadata"], json!({}));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({
        "calls": [{"function": "search", "other": 1}],
    }));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}
