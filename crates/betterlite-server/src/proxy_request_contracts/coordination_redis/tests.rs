#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{BoolOrText, CoordinationRedisNode, CoordinationRedisParams, SentinelValue};
use crate::proxy_request_contracts::tests::{assert_decode, contract};

#[rstest]
#[case::empty("{}")]
#[case::port_only(r#"{"port": 6379}"#)]
#[case::credentials_only(r#"{"username": "u", "password": "p", "ssl": true}"#)]
fn has_no_target_without_a_connection_field(#[case] text: &str) {
    let params = CoordinationRedisParams::from_value(&contract(text)).expect("a params object");
    assert!(!params.has_connection_target());
}

#[rstest]
#[case::host(r#"{"host": ""}"#)]
#[case::url(r#"{"url": ""}"#)]
#[case::startup_nodes(r#"{"startup_nodes": []}"#)]
#[case::sentinel_nodes(r#"{"sentinel_nodes": []}"#)]
fn counts_an_empty_value_as_a_target(#[case] text: &str) {
    let params = CoordinationRedisParams::from_value(&contract(text)).expect("a params object");
    assert!(params.has_connection_target());
}

#[rstest]
#[case::host(r#"{"host": "h", "port": 6379}"#)]
#[case::url(r#"{"url": "redis://h:6379"}"#)]
#[case::startup_nodes(r#"{"startup_nodes": [{"host": "h", "port": 6379}]}"#)]
#[case::sentinel_nodes(r#"{"sentinel_nodes": [["h", 26379]]}"#)]
fn finds_a_target_in_each_connection_field(#[case] text: &str) {
    let params = CoordinationRedisParams::from_value(&contract(text)).expect("a params object");
    assert!(params.has_connection_target());
}

#[test]
fn keeps_unknown_keys() {
    let params = CoordinationRedisParams::from_value(&contract(
        r#"{"host": "h", "cluster_mode": true, "nested": {"a": [1]}}"#,
    ))
    .expect("a params object");
    let expected = BTreeMap::from([
        ("cluster_mode".to_owned(), contract("true")),
        ("nested".to_owned(), contract(r#"{"a": [1]}"#)),
    ]);
    assert_eq!(params.extra, expected);
    let text = serde_json::to_string(&params).expect("the dump");
    assert!(text.contains(r#""cluster_mode":true"#));
    assert!(text.contains(r#""nested":{"a":[1]}"#));
}

#[test]
fn hides_the_passwords_and_url_in_debug() {
    let params = CoordinationRedisParams::from_value(&contract(
        r#"{"password": "hunter2", "sentinel_password": "sentinel-secret", "url": "redis://:hunter3@host:6379"}"#,
    ))
    .expect("a params object");
    let text = format!("{params:?}");
    assert!(!text.contains("hunter2"));
    assert!(!text.contains("sentinel-secret"));
    assert!(!text.contains("hunter3"));
}

#[test]
fn writes_the_passwords_and_url_on_serialize() {
    let params = CoordinationRedisParams::from_value(&contract(
        r#"{"password": "hunter2", "sentinel_password": "sentinel-secret", "url": "redis://:hunter3@host:6379"}"#,
    ))
    .expect("a params object");
    let text = serde_json::to_string(&params).expect("the dump");
    assert!(text.contains(r#""password":"hunter2""#));
    assert!(text.contains(r#""sentinel_password":"sentinel-secret""#));
    assert!(text.contains(r#""url":"redis://:hunter3@host:6379""#));
}

#[test]
fn requires_host_and_port_in_a_node() {
    let missing =
        CoordinationRedisNode::from_value(&contract(r#"{"host": "h"}"#)).expect_err("no port");
    assert_decode(missing);
    let node = CoordinationRedisNode::from_value(&contract(r#"{"host": "h", "port": 6379}"#))
        .expect("a node");
    assert_eq!(node.host, "h");
    assert_eq!(node.port, 6379);
}

#[rstest]
#[case::enabled(r#"{"aws_iam_auth": true}"#, BoolOrText::Bool(true))]
#[case::text(r#"{"aws_iam_auth": "true"}"#, BoolOrText::Text("true".to_owned()))]
#[case::serverless(r#"{"aws_iam_serverless": false}"#, BoolOrText::Bool(false))]
fn accepts_a_bool_or_text_iam_flag(#[case] text: &str, #[case] expected: BoolOrText) {
    let params = CoordinationRedisParams::from_value(&contract(text)).expect("an IAM flag");
    let found = params
        .aws_iam_auth
        .or(params.aws_iam_serverless)
        .expect("the IAM flag");
    assert_eq!(found, expected);
}

#[test]
fn accepts_text_and_integer_sentinel_values() {
    let params = CoordinationRedisParams::from_value(&contract(
        r#"{"sentinel_nodes": [["h", 26379], ["h2", 26380]]}"#,
    ))
    .expect("sentinel nodes");
    let text = serde_json::to_string(&params).expect("the dump");
    assert!(text.contains(r#""sentinel_nodes":[["h",26379],["h2",26380]]"#));
}

#[rstest]
#[case::boolean(r#"{"sentinel_nodes": [[true]]}"#)]
#[case::fraction(r#"{"sentinel_nodes": [[1.5]]}"#)]
#[case::fractional_port(r#"{"port": 6379.5}"#)]
#[case::host_number(r#"{"host": 5}"#)]
fn rejects_a_value_of_the_wrong_type(#[case] text: &str) {
    let error = CoordinationRedisParams::from_value(&contract(text)).expect_err("a typed field");
    assert_decode(error);
}

#[test]
fn serializes_a_sentinel_value_as_text_or_integer() {
    let number = serde_json::to_string(&SentinelValue::Integer(1)).expect("the dump");
    assert_eq!(number, "1");
    let text = serde_json::to_string(&SentinelValue::Text("h".to_owned())).expect("the dump");
    assert_eq!(text, r#""h""#);
}
