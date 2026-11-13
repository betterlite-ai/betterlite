#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn trace_metadata_serializes_the_object_to_json_text() {
    assert_eq!(
        metadata_attributes(r#"{"trace_metadata":{"team":"core"}}"#),
        vec![attr("langfuse.trace.metadata", r#"{"team":"core"}"#)]
    );
}
