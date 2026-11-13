#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn generation_id_writes_the_generation_id() {
    assert_eq!(
        metadata_attributes(r#"{"generation_id":"gen-1"}"#),
        vec![attr("langfuse.generation.id", "gen-1")]
    );
}
