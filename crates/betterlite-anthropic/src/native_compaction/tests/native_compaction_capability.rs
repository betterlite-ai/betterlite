#![cfg(test)]

use rstest::rstest;

use crate::native_compaction::protocol::{CompactionParams, get_native_compaction_provider};

use super::fixtures::{FakeFailure, FakeLookup};

#[rstest]
#[case::plain_model_no_provider("claude-opus-4-7", None, "claude-opus-4-7")]
#[case::anthropic_provider("claude-opus-4-7", Some("anthropic"), "claude-opus-4-7")]
#[case::openai_provider("claude-opus-4-7", Some("openai"), "claude-opus-4-7")]
#[case::anthropic_prefix("anthropic/claude-opus-4-7", None, "claude-opus-4-7")]
#[case::openai_prefix("openai/claude-opus-4-7", None, "claude-opus-4-7")]
#[case::openai_then_anthropic_prefix("openai/anthropic/claude-opus-4-7", None, "claude-opus-4-7")]
#[case::anthropic_then_openai_prefix(
    "anthropic/openai/claude-opus-4-7",
    None,
    "openai/claude-opus-4-7"
)]
#[case::double_anthropic_prefix(
    "anthropic/anthropic/claude-opus-4-7",
    None,
    "anthropic/claude-opus-4-7"
)]
#[case::other_prefix_kept("vertex_ai/claude-opus-4-7", None, "vertex_ai/claude-opus-4-7")]
#[tokio::test]
async fn native_compaction_capability_selects_provider(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] looked_up: &str,
) {
    let lookup = FakeLookup::answering(Ok(Some(true)));
    let params = CompactionParams {
        model,
        custom_llm_provider,
    };

    let provider = get_native_compaction_provider(&lookup, &params).await;

    assert!(provider.is_some());
    assert_eq!(
        lookup.recorded(),
        vec![(looked_up.to_owned(), "anthropic".to_owned())]
    );
}

#[rstest]
#[case::flag_false(Ok(Some(false)))]
#[case::flag_absent(Ok(None))]
#[case::lookup_error(Err(FakeFailure))]
#[tokio::test]
async fn native_compaction_capability_needs_an_explicit_true(
    #[case] answer: Result<Option<bool>, FakeFailure>,
) {
    let lookup = FakeLookup::answering(answer);
    let params = CompactionParams {
        model: "claude-opus-4-7",
        custom_llm_provider: None,
    };

    let provider = get_native_compaction_provider(&lookup, &params).await;

    assert!(provider.is_none());
    assert_eq!(lookup.recorded().len(), 1);
}

#[rstest]
#[case::bedrock("bedrock")]
#[case::vertex("vertex_ai")]
#[case::empty("")]
#[case::wrong_case("Anthropic")]
#[tokio::test]
async fn native_compaction_capability_rejects_other_providers_without_lookup(
    #[case] custom_llm_provider: &str,
) {
    let lookup = FakeLookup::answering(Ok(Some(true)));
    let params = CompactionParams {
        model: "claude-opus-4-7",
        custom_llm_provider: Some(custom_llm_provider),
    };

    let provider = get_native_compaction_provider(&lookup, &params).await;

    assert!(provider.is_none());
    assert!(lookup.recorded().is_empty());
}
