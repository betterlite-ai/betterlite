#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::Value;

use crate::search::{SearchResponse, SearchResult};

#[test]
fn preserves_extra_fields_and_search_default() {
    let response: SearchResponse = serde_json::from_str(
        r#"{"results":[{"title":"A","url":"https://a","snippet":"B","rank":1}],"provider":"demo"}"#,
    )
    .unwrap();
    assert_eq!(
        response.results.first().unwrap().extra["rank"],
        Value::from(1_i32)
    );
    assert_eq!(response.extra["provider"], Value::from("demo"));
    assert_eq!(response.object, "search");
}

#[test]
fn hidden_metadata_is_not_serialized() {
    let mut response = SearchResponse::with_results(vec![SearchResult {
        title: String::from("A"),
        url: String::from("https://a"),
        snippet: String::from("B"),
        date: None,
        last_updated: None,
        extra: BTreeMap::new(),
    }]);
    response.set_hidden_params(BTreeMap::from([(String::from("trace"), Value::Bool(true))]));
    let serialized = serde_json::to_value(response).unwrap();
    assert!(serialized.get("_hidden_params").is_none());
}
