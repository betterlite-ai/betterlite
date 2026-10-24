#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::Value;

use crate::case::{Case, Compare, Ignore};
use crate::compare;
use crate::http::Response;
use crate::mock::ProviderRequest;

fn response(body: Value, headers: &[(&str, &str)]) -> Response {
    Response {
        status: 200,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body,
    }
}

fn request(body: Value) -> ProviderRequest {
    ProviderRequest {
        method: "POST".to_owned(),
        path: "/chat/completions".to_owned(),
        headers: BTreeMap::new(),
        body,
    }
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("test JSON is valid")
}

#[test]
fn case_rejects_unknown_fields() {
    let yaml = "name: test\nconfig: x\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\nextra: true\n";
    let error = serde_yaml::from_str::<Case>(yaml).expect_err("unknown field should fail");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn case_rejects_missing_fields() {
    let yaml = "name: test\nconfig: x\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\n";
    let error = serde_yaml::from_str::<Case>(yaml).expect_err("missing field should fail");
    assert!(error.to_string().contains("missing field"));
}

#[test]
fn case_rejects_duplicate_fields() {
    let yaml = "name: test\nname: other\nconfig: x\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";
    let error = serde_yaml::from_str::<Case>(yaml).expect_err("duplicate field should fail");
    assert!(error.to_string().contains("duplicate field"));
}

#[test]
fn comparator_honors_ignore_and_header_rules() {
    let left_response = response(
        value(r#"{"id":"one","stable":"same"}"#),
        &[("content-type", "application/json")],
    );
    let right_response = response(
        value(r#"{"id":"two","stable":"same"}"#),
        &[("content-type", "application/json")],
    );
    let compare = Compare {
        headers: vec!["content-type".to_owned()],
        headers_present: vec!["x-call-id".to_owned()],
        upstream_headers: vec!["authorization".to_owned()],
        ignore: vec![Ignore {
            pointer: "/id".to_owned(),
            why: "generated".to_owned(),
        }],
    };
    let left = Response {
        headers: {
            let mut headers = left_response.headers;
            headers.insert("x-call-id".to_owned(), "one".to_owned());
            headers
        },
        ..left_response
    };
    let right = Response {
        headers: {
            let mut headers = right_response.headers;
            headers.insert("x-call-id".to_owned(), "two".to_owned());
            headers
        },
        ..right_response
    };
    compare::compare(
        &left,
        &right,
        &[request(value(r#"{"x":1}"#))],
        &[request(value(r#"{"x":1}"#))],
        &compare,
    )
    .expect("ignored response fields and selected headers should match");
}

#[test]
fn comparator_reports_first_pointer() {
    let left = response(
        value(r#"{"choices":[{"message":{"content":"left"}}]}"#),
        &[],
    );
    let right = response(
        value(r#"{"choices":[{"message":{"content":"right"}}]}"#),
        &[],
    );
    let error =
        compare::compare(&left, &right, &[], &[], &Compare::default()).expect_err("body differs");
    assert_eq!(error.pointer, "/choices/0/message/content");
}
