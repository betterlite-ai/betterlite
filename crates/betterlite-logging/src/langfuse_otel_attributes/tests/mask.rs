#![cfg(test)]

use super::support::{flag, metadata_attributes};

#[test]
fn mask_input_writes_the_generation_mask_input() {
    assert_eq!(
        metadata_attributes(r#"{"mask_input":true}"#),
        vec![flag("langfuse.generation.mask_input", true)]
    );
}

#[test]
fn mask_output_writes_the_generation_mask_output() {
    assert_eq!(
        metadata_attributes(r#"{"mask_output":false}"#),
        vec![flag("langfuse.generation.mask_output", false)]
    );
}
