#![cfg(test)]

use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::search::records::{ListSearchToolsResponse, SearchObject, SearchResponse, SearchTool};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn keeps_unknown_fields_and_defaults_object_to_search() {
    let response: SearchResponse = serde_json::from_str(
        r#"{"results": [{"title": "A", "url": "https://a.example", "snippet": "B", "rank": 1}],
        "provider": "demo"}"#,
    )
    .unwrap();
    assert_eq!(response.object, SearchObject(String::from("search")));
    assert_eq!(response.extra["provider"], value(r#""demo""#));
    let result = response.results.first().unwrap();
    assert_eq!(result.extra["rank"], value("1"));
    assert_eq!((&result.date, &result.last_updated), (&None, &None));
}

#[test]
fn writes_absent_dates_as_null_like_model_dump() {
    let response: SearchResponse = serde_json::from_str(
        r#"{"results": [{"title": "A", "url": "https://a.example", "snippet": "B", "date": "2025-01-02"}],
        "object": "search"}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        value(
            r#"{"results": [{"title": "A", "url": "https://a.example", "snippet": "B",
            "date": "2025-01-02", "last_updated": null}], "object": "search"}"#
        )
    );
}

#[test]
fn hidden_params_never_reach_the_wire() {
    let mut response: SearchResponse = serde_json::from_str(r#"{"results": []}"#).unwrap();
    response
        .hidden_params
        .insert(String::from("response_cost"), value("0.002"));
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        value(r#"{"results": [], "object": "search"}"#)
    );
}

#[test]
fn search_tool_reads_config_entry() {
    let tool: SearchTool = serde_json::from_str(
        r#"{"search_tool_name": "litellm-search",
        "litellm_params": {"search_provider": "google_pse", "api_key": "sk-search",
            "timeout": 2.5, "max_retries": 3, "search_engine_id": "cx-1"},
        "search_tool_info": {"description": "Perplexity search tool"}}"#,
    )
    .unwrap();
    let params = &tool.litellm_params;
    assert_eq!(params.search_provider, "google_pse");
    assert_eq!(
        params.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-search")
    );
    assert_eq!(params.timeout, Some(Decimal::new(25, 1)));
    assert_eq!(params.max_retries, Some(3));
    assert_eq!(params.extra["search_engine_id"], value(r#""cx-1""#));
    assert_eq!(tool.search_tool_id, None);
    assert!(!format!("{tool:?}").contains("sk-search"));
}

#[test]
fn search_tool_names_an_unknown_key() {
    let error = serde_json::from_str::<SearchTool>(
        r#"{"search_tool_name": "litellm-search",
        "litellm_params": {"search_provider": "tavily"}, "owner": "team-a"}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("owner"));
}

#[test]
fn list_response_round_trips_without_absent_keys() {
    let body = value(
        r#"{"search_tools": [{"search_tool_id": "123e4567-e89b-12d3-a456-426614174000",
        "search_tool_name": "litellm-search", "litellm_params": {"search_provider": "perplexity"},
        "is_from_config": true}]}"#,
    );
    let list: ListSearchToolsResponse = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(
        list.search_tools.first().unwrap().is_from_config,
        Some(true)
    );
    assert_eq!(serde_json::to_value(list).unwrap(), body);
}
