#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn generation_name_writes_the_generation_name() {
    assert_eq!(
        metadata_attributes(r#"{"generation_name":"planning"}"#),
        vec![attr("langfuse.generation.name", "planning")]
    );
}

#[test]
fn a_null_generation_name_writes_nothing() {
    assert!(metadata_attributes(r#"{"generation_name":null}"#).is_empty());
}
