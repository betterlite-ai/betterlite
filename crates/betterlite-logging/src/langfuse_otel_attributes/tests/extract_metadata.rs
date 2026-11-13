#![cfg(test)]

use super::metadata::extract_langfuse_metadata;
use super::support::value;

#[test]
fn reads_metadata_from_litellm_params() {
    let metadata = extract_langfuse_metadata(&value(
        r#"{"litellm_params":{"metadata":{"session_id":"s1","tags":["a"]}}}"#,
    ));
    assert_eq!(metadata.get("session_id"), Some(&value(r#""s1""#)));
    assert_eq!(metadata.get("tags"), Some(&value(r#"["a"]"#)));
}

#[test]
fn missing_litellm_params_gives_an_empty_metadata() {
    assert!(extract_langfuse_metadata(&value("{}")).is_empty());
    assert!(extract_langfuse_metadata(&value(r#"{"litellm_params":null}"#)).is_empty());
}

#[test]
fn non_object_metadata_gives_an_empty_metadata() {
    assert!(
        extract_langfuse_metadata(&value(r#"{"litellm_params":{"metadata":"text"}}"#)).is_empty()
    );
}
