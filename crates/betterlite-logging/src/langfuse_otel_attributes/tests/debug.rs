#![cfg(test)]

use super::support::{flag, metadata_attributes};

#[test]
fn debug_langfuse_writes_the_debug_flag() {
    assert_eq!(
        metadata_attributes(r#"{"debug_langfuse":true}"#),
        vec![flag("langfuse.debug", true)]
    );
}
