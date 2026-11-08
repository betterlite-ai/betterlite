#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn parent_observation_id_keeps_its_dashes() {
    assert_eq!(
        metadata_attributes(r#"{"parent_observation_id":"parent-1"}"#),
        vec![attr(
            "langfuse.generation.parent_observation_id",
            "parent-1"
        )]
    );
}
