#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

fn run_error(value: Value) -> HelperError {
    let input = serde_json::from_value(value).unwrap();
    *super::ops::run(&input).unwrap_err()
}

#[rstest]
fn helper_seeds_select_vertex_discovery_search(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/vertex_discovery_search*")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_discovery_search");
}

#[rstest]
fn matches_pinned_vertex_discovery_search(
    #[base_dir = "../../.."]
    #[files("cases/helpers/vertex-discovery-search/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}

#[test]
fn a_target_call_builds_the_engine_url_with_the_default_collection() {
    let value = run(json!({
        "calls": [{
            "function": "target",
            "litellm_params": {
                "vertex_project": "project-example",
                "vertex_location": "us-central1",
                "vertex_engine_id": "app-example",
            },
        }],
    }));
    assert_eq!(
        value["calls"][0]["url"],
        json!(concat!(
            "https://discoveryengine.googleapis.com/v1/projects/project-example",
            "/locations/us-central1/collections/default_collection",
            "/engines/app-example/servingConfigs/default_serving_config",
        ))
    );
}

#[test]
fn a_datastore_target_without_a_store_names_the_missing_field() {
    let error = run_error(json!({
        "calls": [{
            "function": "target",
            "litellm_params": {
                "vertex_project": "project-example",
                "vertex_location": "us-central1",
            },
        }],
    }));
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
    assert!(error.message.contains("vector_store_id"));
}

#[test]
fn a_target_selecting_field_fails_with_the_bad_request_status() {
    let error = run_error(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "extra_body": {"servingConfig": "other"},
        }],
    }));
    assert_eq!(error.class, "BadRequestError");
    assert_eq!(error.status, Some(400));
    assert_eq!(error.error_type, Some(None));
}

#[test]
fn a_null_target_selecting_field_leaves_the_body_alone() {
    let value = run(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "extra_body": {"servingConfig": null, "filter": "active"},
        }],
    }));
    assert_eq!(value["calls"][0]["url"], json!("https://host/v1:search"));
    assert_eq!(
        value["calls"][0]["body"],
        json!({"query": "policy", "pageSize": 10_i64, "filter": "active"})
    );
}

#[test]
fn a_search_page_reports_six_decimal_scores_and_the_logged_query() {
    let value = run(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "extra_body": {"query": "native policy"},
            "response": {
                "status": 200_u16,
                "body": r#"{"results":[{"id":"doc-1"},{"id":"doc-2"}]}"#,
            },
        }],
    }));
    let results = &value["calls"][0]["results"];
    assert_eq!(results["search_query"], json!("native policy"));
    assert_eq!(results["data"][0]["score"], json!("1.000000"));
    assert_eq!(results["data"][1]["score"], json!("0.500000"));
}

#[test]
fn a_chunk_score_overrides_the_position() {
    let value = run(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "response": {
                "status": 200_u16,
                "body": r#"{"results":[{"chunk":{"content":"body","relevanceScore":0.25}}]}"#,
            },
        }],
    }));
    assert_eq!(
        value["calls"][0]["results"]["data"][0]["score"],
        json!("0.250000")
    );
}

#[test]
fn a_native_query_that_is_not_text_names_the_field() {
    let error = run_error(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "extra_body": {"query": 7_i64},
        }],
    }));
    assert_eq!(error.class, "ValueError");
    assert!(error.message.contains("query"));
}

#[test]
fn a_decode_failure_keeps_the_reply_status() {
    let error = run_error(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "response": {"status": 502_u16, "body": "not json"},
        }],
    }));
    assert_eq!(error.class, "BaseLLMException");
    assert_eq!(error.status, Some(502));
}

#[test]
fn a_query_cost_reports_the_catalog_price_and_no_output_cost() {
    let value = run(json!({"calls": [{"function": "cost"}]}));
    assert_eq!(
        value["calls"][0],
        json!({"input": "0.001500", "output": "0.000000"})
    );
}

#[test]
fn an_unknown_call_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({
        "calls": [{
            "function": "search",
            "query": "policy",
            "api_base": "https://host/v1",
            "other": 1_i32,
        }],
    }));
    assert!(
        unknown
            .err()
            .is_some_and(|error| error.to_string().contains("other"))
    );
}
