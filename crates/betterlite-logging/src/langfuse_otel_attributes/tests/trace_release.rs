#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn trace_release_writes_the_release() {
    assert_eq!(
        metadata_attributes(r#"{"trace_release":"v1.2.3"}"#),
        vec![attr("langfuse.release", "v1.2.3")]
    );
}
