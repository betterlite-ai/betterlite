#![cfg(test)]

use super::support::{attr, observation_attributes};

#[test]
fn writes_every_reasoning_summary() {
    assert_eq!(
        observation_attributes(
            r#"{"output":[{"type":"reasoning","summary":[{"text":"one"},{"text":"two"}]}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"content":"one","role":"reasoning_summary"},{"content":"two","role":"reasoning_summary"}]"#
        )]
    );
}

#[test]
fn a_reasoning_item_without_summaries_writes_nothing() {
    assert!(observation_attributes(r#"{"output":[{"type":"reasoning"}]}"#).is_empty());
}
