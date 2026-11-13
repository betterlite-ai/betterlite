#![cfg(test)]

use super::metadata::extract_langfuse_metadata;
use super::support::value;

#[test]
fn failed_header_enrichment_keeps_the_base_metadata() {
    let metadata = extract_langfuse_metadata(&value(
        r#"{"litellm_params":{"metadata":{"session_id":"s1"},"proxy_server_request":"unavailable"}}"#,
    ));
    assert_eq!(metadata.get("session_id"), Some(&value(r#""s1""#)));
}

#[test]
fn failed_header_enrichment_never_falls_back_to_a_default_metadata() {
    let metadata = extract_langfuse_metadata(&value(
        r#"{"litellm_params":{"proxy_server_request":{"headers":"unavailable"}}}"#,
    ));
    assert!(metadata.is_empty());
}
