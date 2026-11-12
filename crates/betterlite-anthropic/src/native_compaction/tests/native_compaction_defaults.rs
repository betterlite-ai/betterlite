#![cfg(test)]

use rstest::rstest;

use crate::native_compaction::anthropic::AnthropicCompaction;
use crate::native_compaction::protocol::NativeCompactionProvider as _;

use super::fixtures::{FakeLookup, mapping};

#[rstest]
#[case::empty("{}", true)]
#[case::unrelated_keys(
    r#"{"model":"m","messages":[],"temperature":0.2,"max_tokens":0}"#,
    true
)]
#[case::nulls(
    r#"{"context_management":null,"response_format":null,"stop":null,"stop_sequences":null,"tool_choice":null}"#,
    true
)]
#[case::context_management(r#"{"context_management":{}}"#, false)]
#[case::response_format(r#"{"response_format":{"type":"json_object"}}"#, false)]
#[case::stop_text(r#"{"stop":"END"}"#, false)]
#[case::stop_empty_list(r#"{"stop":[]}"#, false)]
#[case::stop_sequences(r#"{"stop_sequences":["a"]}"#, false)]
#[case::stop_sequences_empty_text(r#"{"stop_sequences":""}"#, false)]
#[case::tool_choice_text(r#"{"tool_choice":"auto"}"#, false)]
#[case::tool_choice_false(r#"{"tool_choice":false}"#, false)]
#[case::tool_choice_zero(r#"{"tool_choice":0}"#, false)]
#[case::one_conflict_among_others(r#"{"model":"m","stop":"x","messages":[]}"#, false)]
fn native_compaction_defaults_accept_only_unset_conflicts(
    #[case] payload: &str,
    #[case] compatible: bool,
) {
    let lookup = FakeLookup::answering(Ok(Some(true)));
    let provider = AnthropicCompaction { lookup: &lookup };

    assert_eq!(provider.compatible_defaults(&mapping(payload)), compatible);
}
