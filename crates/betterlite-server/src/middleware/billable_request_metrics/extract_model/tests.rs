#![cfg(test)]

use axum::http::HeaderMap;

use super::extract_model_id;

#[test]
fn reads_the_model_id_header() {
    let mut headers = HeaderMap::new();
    headers.insert("x-litellm-model-id", "gpt-4o-mini".parse().unwrap());
    assert_eq!(extract_model_id(&headers), Some("gpt-4o-mini".to_owned()));
}

#[test]
fn reads_the_model_id_header_case_insensitively() {
    let mut headers = HeaderMap::new();
    headers.insert("X-LiteLLM-Model-Id", "gpt-4o-mini".parse().unwrap());
    assert_eq!(extract_model_id(&headers), Some("gpt-4o-mini".to_owned()));
}

#[test]
fn returns_none_when_header_is_absent() {
    let headers = HeaderMap::new();
    assert_eq!(extract_model_id(&headers), None);
}

#[test]
fn returns_none_when_header_is_empty() {
    let mut headers = HeaderMap::new();
    headers.insert("x-litellm-model-id", "".parse().unwrap());
    assert_eq!(extract_model_id(&headers), None);
}
