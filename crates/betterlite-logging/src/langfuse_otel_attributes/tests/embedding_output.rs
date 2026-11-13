#![cfg(test)]

use super::output_content::get_output_content_by_type;
use super::support::value;
use crate::langfuse_projections::content::ResponseObj;

#[test]
fn embedding_response_returns_embedding_output() {
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Embedding), None),
        "embedding-output"
    );
}

#[test]
fn embedding_call_type_returns_embedding_output() {
    let body = value(r#"{"data":[]}"#);
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Dict(&body)), Some("embedding")),
        "embedding-output"
    );
}

#[test]
fn missing_response_returns_empty_even_for_an_embedding_call() {
    assert_eq!(get_output_content_by_type(None, Some("embedding")), "");
}
