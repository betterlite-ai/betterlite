#![cfg(test)]

use super::metadata::extract_langfuse_metadata;
use super::support::value;

#[test]
fn header_values_replace_existing_metadata() {
    let metadata = extract_langfuse_metadata(&value(
        r#"{"litellm_params":{"metadata":{"session_id":"old"},"proxy_server_request":{"headers":{"langfuse_session_id":"new"}}}}"#,
    ));
    assert_eq!(metadata.get("session_id"), Some(&value(r#""new""#)));
}

#[test]
fn header_values_add_new_keys() {
    let metadata = extract_langfuse_metadata(&value(
        r#"{"litellm_params":{"proxy_server_request":{"headers":{"langfuse_trace_name":"nightly"}}}}"#,
    ));
    assert_eq!(metadata.get("trace_name"), Some(&value(r#""nightly""#)));
}
