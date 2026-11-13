#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn existing_trace_id_writes_the_existing_trace_id() {
    assert_eq!(
        metadata_attributes(r#"{"existing_trace_id":"trace-1"}"#),
        vec![attr("langfuse.trace.existing_id", "trace-1")]
    );
}
