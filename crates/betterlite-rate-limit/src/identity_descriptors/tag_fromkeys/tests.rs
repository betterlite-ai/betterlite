#![cfg(test)]

use betterlite_core::api::request_destination::RequestData;

use super::deduplicated_tags;

fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("the request body fixture decodes")
}

#[test]
fn deduplicates_tags_across_metadata_and_body() {
    let body = data(r#"{"metadata": {"tags": ["a", "b", "a"]}, "tags": ["b", "c"]}"#);
    assert_eq!(deduplicated_tags(&body), vec!["a", "b", "c"]);
}

#[test]
fn keeps_metadata_tags_before_body_tags() {
    let body = data(r#"{"metadata": {"tags": ["m1", "m2"]}, "tags": ["b1"]}"#);
    assert_eq!(deduplicated_tags(&body), vec!["m1", "m2", "b1"]);
}

#[test]
fn deduplicates_a_tag_repeated_in_the_body() {
    let body = data(r#"{"tags": ["x", "x", "x"]}"#);
    assert_eq!(deduplicated_tags(&body), vec!["x"]);
}

#[test]
fn returns_no_tags_when_none_are_sent() {
    let body = data(r#"{"model": "gpt-4"}"#);
    assert!(deduplicated_tags(&body).is_empty());
}

#[test]
fn drops_non_text_tags() {
    let body = data(r#"{"tags": [1, "a", null, "b"]}"#);
    assert_eq!(deduplicated_tags(&body), vec!["a", "b"]);
}
