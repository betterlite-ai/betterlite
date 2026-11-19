use rstest::rstest;

use super::super::chunk::{ChunkError, ChunkPlan};
use super::*;

#[rstest]
#[case::zero(0)]
#[case::negative(-5)]
#[test]
fn chunk_config_rejects_a_non_positive_size(#[case] size: i64) {
    let parsed = serde_json::from_str(&format!(
        r#"{{"chunking_strategy": {{"chunk_size": {size}}}}}"#
    ))
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .expect_err("the size is rejected");
    assert!(matches!(error, ChunkError::Configuration(_)));
}

#[test]
fn chunk_config_rejects_a_negative_overlap() {
    let parsed =
        serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 10, "chunk_overlap": -1}}"#)
            .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .expect_err("the overlap is rejected");
    assert!(matches!(error, ChunkError::Configuration(_)));
}

#[rstest]
#[case::equal(10)]
#[case::greater(11)]
#[test]
fn chunk_config_rejects_an_overlap_at_or_above_the_size(#[case] overlap: i64) {
    let parsed = serde_json::from_str(&format!(
        r#"{{"chunking_strategy": {{"chunk_size": 10, "chunk_overlap": {overlap}}}}}"#
    ))
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .expect_err("the overlap is rejected");
    assert!(matches!(error, ChunkError::Configuration(_)));
}

#[test]
fn chunk_config_accepts_an_overlap_just_below_the_size() {
    let parsed =
        serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 10, "chunk_overlap": 9}}"#)
            .expect("the options parse");
    let state = Harness::default().state(parsed);
    let chunks = state.chunk(Some("short"), None, false).expect("chunks");
    assert_eq!(chunks, ["short"]);
}

#[test]
fn chunk_config_null_values_take_the_defaults() {
    let parsed = serde_json::from_str(
        r#"{"chunking_strategy": {"chunk_size": null, "chunk_overlap": null, "separators": null}}"#,
    )
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    let plan = ChunkPlan::from_strategy(&state.chunking_strategy).expect("the plan");
    assert_eq!(plan.size, 1000);
    assert_eq!(plan.overlap, 200);
    assert_eq!(plan.separators, ["\n\n", "\n", " ", ""]);
}

#[rstest]
#[case::size_text(r#"{"chunk_size": "5"}"#)]
#[case::overlap_text(r#"{"chunk_overlap": "5"}"#)]
#[case::separators_text(r#"{"separators": "x"}"#)]
#[test]
fn chunk_config_wrong_types_are_rejected(#[case] strategy: &str) {
    let parsed = serde_json::from_str(&format!(r#"{{"chunking_strategy": {strategy}}}"#))
        .expect("the options parse");
    let state = Harness::default().state(parsed);
    let error = state
        .chunk(Some("text"), None, false)
        .expect_err("the types are rejected");
    assert!(matches!(error, ChunkError::Strategy(_)));
}

#[test]
fn chunk_config_is_not_validated_without_text() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 0}}"#)
        .expect("the options parse");
    let state = Harness::default().state(parsed);
    assert!(state.chunk(None, None, false).expect("chunks").is_empty());
}
