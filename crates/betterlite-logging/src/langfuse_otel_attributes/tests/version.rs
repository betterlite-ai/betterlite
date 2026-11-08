#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn trace_version_wins_over_version() {
    assert_eq!(
        metadata_attributes(r#"{"trace_version":"2.0","version":"1.0"}"#),
        vec![attr("langfuse.version", "2.0")]
    );
}

#[test]
fn version_is_the_fallback() {
    assert_eq!(
        metadata_attributes(r#"{"version":"1.0"}"#),
        vec![attr("langfuse.version", "1.0")]
    );
}

#[test]
fn a_null_trace_version_falls_back_to_version() {
    assert_eq!(
        metadata_attributes(r#"{"trace_version":null,"version":"1.0"}"#),
        vec![attr("langfuse.version", "1.0")]
    );
}

#[test]
fn no_version_writes_nothing() {
    assert!(metadata_attributes(r#"{"version":null}"#).is_empty());
}
