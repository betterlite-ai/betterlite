use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rstest::rstest;

use super::super::chunk::{ChunkError, ChunkPlan};
use super::*;

#[test]
fn chunk_splits_the_given_text() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(Some("hello world"), None, true)
        .expect("chunks");
    assert_eq!(chunks, ["hello world"]);
}

#[test]
fn chunk_prefers_the_text_over_the_file_content() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(Some("from ocr"), Some(b"from file"), true)
        .expect("chunks");
    assert_eq!(chunks, ["from ocr"]);
}

#[test]
fn chunk_decodes_utf8_file_content() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(None, Some(b"from file"), false)
        .expect("chunks");
    assert_eq!(chunks, ["from file"]);
}

#[test]
fn chunk_ignores_file_content_when_ocr_was_used() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state.chunk(None, Some(b"from file"), true).expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_extracts_text_from_a_binary_pdf() {
    let state = Harness::default().state(IngestionOptions::default());
    let pdf = sample_pdf();
    let chunks = state
        .chunk(None, Some(pdf.as_slice()), false)
        .expect("chunks");
    assert_eq!(chunks, ["Hello PDF"]);
}

#[test]
fn chunk_skips_binary_content_that_is_not_a_pdf() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(None, Some(&[0xff, 0xfe, 0x00]), false)
        .expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_skips_a_pdf_without_extractable_text() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state
        .chunk(None, Some(b"%PDF-\xff"), false)
        .expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_skips_a_pdf_behind_a_binary_preamble() {
    let state = Harness::default().state(IngestionOptions::default());
    let mut content = vec![0xff, 0xfe];
    content.extend_from_slice(&sample_pdf());
    let chunks = state.chunk(None, Some(&content), false).expect("chunks");
    assert!(chunks.is_empty());
}

#[test]
fn chunk_with_no_source_returns_no_chunks() {
    let state = Harness::default().state(IngestionOptions::default());
    assert!(state.chunk(None, None, false).expect("chunks").is_empty());
    assert!(
        state
            .chunk(None, Some(b""), false)
            .expect("chunks")
            .is_empty()
    );
    assert!(
        state
            .chunk(Some(""), Some(b"from file"), false)
            .expect("chunks")
            .is_empty()
    );
}

#[test]
fn chunk_uses_the_configured_size_and_separators() {
    let state = Harness::default().state(
        serde_json::from_str(
            r#"{"chunking_strategy": {"chunk_size": 5, "chunk_overlap": 0, "separators": ["\n\n", ""]}}"#,
        )
        .expect("the options parse"),
    );
    let chunks = state
        .chunk(Some("one\n\ntwo\n\nthree"), None, false)
        .expect("chunks");
    assert_eq!(chunks, ["one", "two", "three"]);
}

#[test]
fn chunk_defaults_the_size_and_overlap() {
    let state = Harness::default().state(IngestionOptions::default());
    let chunks = state.chunk(Some("abcdefgh"), None, false).expect("chunks");
    assert_eq!(chunks, ["abcdefgh"]);
}

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

fn sample_pdf() -> Vec<u8> {
    let encoded = "JVBERi0xLjQKJenpCjEgMCBvYmoKPDwgL1R5cGUgL0NhdGFsb2cgL1BhZ2VzIDIgMCBSID4+CmVuZG9iagoyIDAgb2JqCjw8IC9UeXBlIC9QYWdlcyAvS2lkcyBbMyAwIFJdIC9Db3VudCAxID4+CmVuZG9iagozIDAgb2JqCjw8IC9UeXBlIC9QYWdlIC9QYXJlbnQgMiAwIFIgL1Jlc291cmNlcyA8PCAvRm9udCA8PCAvRjEgNCAwIFIgPj4gPj4gL01lZGlhQm94IFswIDAgNjEyIDc5Ml0gL0NvbnRlbnRzIDUgMCBSID4+CmVuZG9iago0IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKNSAwIG9iago8PCAvTGVuZ3RoIDQyID4+CnN0cmVhbQpCVCAvRjEgMTIgVGYgMTAwIDcwMCBUZCAoSGVsbG8gUERGKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCnhyZWYKMCA2CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAxMyAwMDAwMCBuIAowMDAwMDAwMDYyIDAwMDAwIG4gCjAwMDAwMDAxMTkgMDAwMDAgbiAKMDAwMDAwMDI0NSAwMDAwMCBuIAowMDAwMDAwMzE1IDAwMDAwIG4gCnRyYWlsZXIKPDwgL1NpemUgNiAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKNDA2CiUlRU9GCg==";
    BASE64.decode(encoded).expect("the fixture decodes")
}
