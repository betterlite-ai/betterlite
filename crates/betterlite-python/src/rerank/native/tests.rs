#![cfg(test)]

use super::{MAX_DOCUMENTS, MAX_HEADERS, MAX_REQUEST_BYTES, bounded_wire};
use crate::rerank::error::SessionError;
use betterlite_models::decisions::Json;
use std::collections::BTreeMap;

fn error_of(text: &str) -> SessionError {
    bounded_wire(text).unwrap_err()
}

fn document_wire(count: usize) -> String {
    let documents = vec!["\"d\""; count].join(",");
    format!("{{\"arguments\": {{\"documents\": [{documents}]}}}}")
}

fn header_object(count: usize) -> String {
    let joined = (0..count)
        .map(|index| format!("\"h{index}\": \"v\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{joined}}}")
}

fn header_wire(module: usize, caller: usize) -> String {
    format!(
        "{{\"settings\": {{\"headers\": {}}}, \"kwargs\": {{\"headers\": {}}}}}",
        header_object(module),
        header_object(caller)
    )
}

#[test]
fn request_caps_keep_their_values() {
    assert_eq!(MAX_REQUEST_BYTES, 0x0080_0000);
    assert_eq!(MAX_DOCUMENTS, 100_000);
    assert_eq!(MAX_HEADERS, 256);
}

#[test]
fn request_at_the_byte_limit_parses() {
    let prefix = "{\"arguments\": {\"query\": \"";
    let suffix = "\"}}";
    let padding = MAX_REQUEST_BYTES
        .saturating_sub(prefix.len())
        .saturating_sub(suffix.len());
    let text = format!("{prefix}{}{suffix}", "x".repeat(padding));
    assert_eq!(text.len(), MAX_REQUEST_BYTES);
    let wire = bounded_wire(&text).unwrap();
    assert_eq!(wire.arguments.query.len(), padding);
}

#[test]
fn request_over_the_byte_limit_is_refused() {
    let count = MAX_REQUEST_BYTES.saturating_add(1);
    let error = error_of(&"x".repeat(count));
    let expected = format!("the rerank request has {count} bytes (limit {MAX_REQUEST_BYTES})");
    assert_eq!(error.to_string(), expected);
}

#[test]
fn documents_at_the_limit_parse() {
    let wire = bounded_wire(&document_wire(MAX_DOCUMENTS)).unwrap();
    assert_eq!(wire.arguments.documents.len(), MAX_DOCUMENTS);
}

#[test]
fn documents_over_the_limit_are_refused() {
    let count = MAX_DOCUMENTS.saturating_add(1);
    let error = error_of(&document_wire(count));
    let expected = format!("the rerank request has {count} documents (limit {MAX_DOCUMENTS})");
    assert_eq!(error.to_string(), expected);
}

#[test]
fn headers_at_the_limit_parse() {
    let both = bounded_wire(&header_wire(MAX_HEADERS.saturating_sub(1), 1)).unwrap();
    assert_eq!(
        both.settings.headers.as_ref().map(BTreeMap::len),
        Some(MAX_HEADERS.saturating_sub(1))
    );
    let caller_only = bounded_wire(&header_wire(0, MAX_HEADERS)).unwrap();
    let Json::Object(caller) = caller_only.kwargs.get("headers").unwrap() else {
        panic!("the caller headers are not an object");
    };
    assert_eq!(caller.len(), MAX_HEADERS);
}

#[test]
fn headers_over_the_limit_are_refused() {
    let count = MAX_HEADERS.saturating_add(1);
    let expected = format!("the rerank request has {count} headers (limit {MAX_HEADERS})");
    assert_eq!(error_of(&header_wire(MAX_HEADERS, 1)).to_string(), expected);
    assert_eq!(error_of(&header_wire(0, count)).to_string(), expected);
    assert_eq!(error_of(&header_wire(count, 0)).to_string(), expected);
}
