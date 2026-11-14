#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

fn input() -> Value {
    json!({
        "model": "semantic-ranker-512@latest",
        "api_base": "https://ignored.example",
        "vertex_project": "param-project",
        "vertex_credentials": "credentials-text",
        "access_project": "access-project",
        "headers": {"X-Trace": "1"},
        "query": "q",
        "documents": ["alpha beta gamma delta", {"text": "body", "title": "name"}],
        "top_n": 1_i64,
        "return_documents": false,
        "non_default_params": {"top_n": 1_i64},
        "litellm_params": null,
        "response": {"records": [{"id": "1", "score": 0.5_f64}, {"id": "0"}]},
    })
}

fn run(value: &Value) -> Value {
    super::Op::decode_and_run(value).unwrap()
}

#[test]
fn helper_seeds_select_vertex_rerank() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_rerank"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        validate_helper(Path::new("<helper>"), seed).unwrap();
        seed.call.operation == "vertex_rerank" && !seed.call.input.is_null()
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let mut value = input();
    value["other"] = json!(1_i64);
    assert!(super::Op::check(&value).unwrap_err().contains("other"));
}

#[test]
fn the_target_ignores_the_base_and_asks_for_the_params() {
    let value = run(&input());
    assert_eq!(
        value["target"]["url"],
        json!(
            "https://discoveryengine.googleapis.com/v1/projects/access-project/locations/global/rankingConfigs/default_ranking_config:rank"
        )
    );
    let lookup = json!({"credentials": "credentials-text", "project": "param-project", "provider": "vertex_ai"});
    assert_eq!(value["target"]["lookups"], json!([lookup.clone(), lookup]));
    assert_eq!(
        value["target"]["headers"]["X-Goog-User-Project"],
        json!("access-project")
    );
    assert_eq!(value["target"]["headers"]["X-Trace"], json!("1"));
}

#[test]
fn the_ranking_orders_by_score_and_bills_the_documents() {
    let value = run(&input());
    assert_eq!(value["rank"]["request"]["topN"], json!(1_i64));
    assert_eq!(
        value["rank"]["request"]["ignoreRecordDetailsInResponse"],
        json!(true)
    );
    assert_eq!(
        value["rank"]["response"]["id"],
        json!("vertex_ai_rerank_00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(
        value["rank"]["response"]["results"],
        json!([
            {"index": 0_i64, "relevance_score": "1"},
            {"index": 1_i64, "relevance_score": "0.5"},
        ])
    );
    assert_eq!(value["rank"]["response"]["search_units"], json!(1_i64));
}
