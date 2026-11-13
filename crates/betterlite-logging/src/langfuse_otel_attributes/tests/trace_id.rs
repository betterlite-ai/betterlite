#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn trace_id_drops_the_dashes() {
    assert_eq!(
        metadata_attributes(r#"{"trace_id":"8f8b1c2e-1234-4abc-9def-0123456789ab"}"#),
        vec![attr(
            "langfuse.trace.id",
            "8f8b1c2e12344abc9def0123456789ab"
        )]
    );
}

#[test]
fn a_non_text_trace_id_is_left_as_given() {
    assert_eq!(
        metadata_attributes(r#"{"trace_id":["a-b"]}"#),
        vec![attr("langfuse.trace.id", r#"["a-b"]"#)]
    );
}
