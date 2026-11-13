#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn trace_name_writes_the_trace_name() {
    assert_eq!(
        metadata_attributes(r#"{"trace_name":"nightly"}"#),
        vec![attr("langfuse.trace.name", "nightly")]
    );
}
