#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn update_trace_keys_writes_the_update_keys() {
    assert_eq!(
        metadata_attributes(r#"{"update_trace_keys":"trace_name"}"#),
        vec![attr("langfuse.trace.update_keys", "trace_name")]
    );
}
