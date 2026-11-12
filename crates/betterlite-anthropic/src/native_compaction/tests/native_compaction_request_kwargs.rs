#![cfg(test)]

use crate::native_compaction::anthropic::{AnthropicCompaction, COMPACT_BETA_HEADER};
use crate::native_compaction::protocol::NativeCompactionProvider as _;

use super::fixtures::{FakeLookup, mapping};

#[test]
fn native_compaction_request_kwargs_match_litellm() {
    let lookup = FakeLookup::answering(Ok(Some(true)));
    let provider = AnthropicCompaction { lookup: &lookup };

    let wire = serde_json::to_string(&provider.request_kwargs()).unwrap();

    assert_eq!(
        mapping(&wire),
        mapping(
            r#"{"compaction":{"type":"summarize"},"extra_headers":{"anthropic-beta":"compact-2026-09-04"}}"#
        )
    );
    assert_eq!(COMPACT_BETA_HEADER, "compact-2026-09-04");
}
