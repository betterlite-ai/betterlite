#![cfg(test)]
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::search::tools::{ListSearchToolsResponse, SearchTool};
fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}
#[test]
fn search_tool_reads_config_entry() {
    let tool: SearchTool = serde_json::from_str(
        r#"{"search_tool_name": "litellm-search",
        "litellm_params": {"search_provider": "google_pse", "api_key": "sk-search",
            "aws_secret_access_key": "aws-secret", "timeout": 2.5, "max_retries": 3,
            "search_engine_id": "cx-1"},
        "search_tool_info": {"description": "Perplexity search tool"}}"#,
    )
    .unwrap();
    let params = &tool.litellm_params;
    let key = params.api_key.as_ref().map(ExposeSecret::expose_secret);
    assert_eq!(
        (params.search_provider.as_str(), key),
        ("google_pse", Some("sk-search"))
    );
    assert_eq!(
        (params.timeout, params.max_retries),
        (Some(Decimal::new(25, 1)), Some(3))
    );
    assert_eq!(params.extra["search_engine_id"], value(r#""cx-1""#));
    let debug = format!("{tool:?}");
    assert!(!debug.contains("sk-search"));
    assert!(!debug.contains("aws-secret"));
    assert!(debug.contains("aws_secret_access_key"));
    let unknown = serde_json::from_str::<SearchTool>(
        r#"{"search_tool_name": "s", "litellm_params": {"search_provider": "tavily"}, "owner": "a"}"#,
    );
    assert!(unknown.unwrap_err().to_string().contains("owner"));
}
#[test]
fn list_response_round_trips_without_absent_keys() {
    let body = value(
        r#"{"search_tools": [{"search_tool_id": "123e4567-e89b-12d3-a456-426614174000",
        "search_tool_name": "litellm-search", "litellm_params": {"search_provider": "perplexity"},
        "is_from_config": true}]}"#,
    );
    let list: ListSearchToolsResponse = serde_json::from_value(body.clone()).unwrap();
    let first = list.search_tools.first().unwrap();
    assert_eq!(first.is_from_config, Some(true));
    assert_eq!(serde_json::to_value(list).unwrap(), body);
}
