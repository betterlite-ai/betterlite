#![cfg(test)]
use serde_json::Value;

use crate::search::records::{SearchObject, SearchResponse};
fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}
#[test]
fn response_keeps_extras_defaults_object_and_hides_metadata() {
    let mut response: SearchResponse = serde_json::from_str(
        r#"{"results": [{"title": "A", "url": "https://a.example", "snippet": "B",
        "date": "2025-01-02", "rank": 1}], "provider": "demo"}"#,
    )
    .unwrap();
    assert_eq!(response.object, SearchObject(String::from("search")));
    let result = response.results.first().unwrap();
    assert_eq!(result.extra["rank"], value("1"));
    response
        .hidden_params
        .insert(String::from("response_cost"), value("0.002"));
    let expected = r#"{"results": [{"title": "A", "url": "https://a.example", "snippet": "B",
        "date": "2025-01-02", "last_updated": null, "rank": 1}], "object": "search", "provider": "demo"}"#;
    assert_eq!(serde_json::to_value(response).unwrap(), value(expected));
}
