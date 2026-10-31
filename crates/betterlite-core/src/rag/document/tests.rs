#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::pdf::extract_text_from_pdf;
use super::provider::{IngestionProviderError, get_ingestion_class};
use super::splitter::{
    ChunkConfigError, DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE, DEFAULT_MAX_RECURSE_DEPTH,
    RecursiveCharacterTextSplitter, TextSplitter as _,
};

fn push_object(buf: &mut Vec<u8>, offsets: &mut [usize], index: usize, body: &str) {
    offsets[index] = buf.len();
    buf.extend_from_slice(format!("{index} 0 obj\n{body}\nendobj\n").as_bytes());
}

fn finish_pdf(buf: &mut Vec<u8>, offsets: &[usize]) {
    let count = offsets.len();
    let xref_offset = buf.len();
    buf.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets[1..] {
        buf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF")
            .as_bytes(),
    );
}

fn build_pdf(object_count: usize, bodies: &[String]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = vec![0_usize; object_count];
    for (zero_based_index, body) in bodies.iter().enumerate() {
        push_object(
            &mut buf,
            &mut offsets,
            zero_based_index.saturating_add(1),
            body,
        );
    }
    finish_pdf(&mut buf, &offsets);
    buf
}

fn pdf_with_stream(stream: &str) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
    ];
    build_pdf(6, &bodies)
}

fn minimal_pdf(text: &str) -> Vec<u8> {
    pdf_with_stream(&format!("BT /F1 12 Tf 100 700 Td ({text}) Tj ET"))
}

fn two_page_pdf(stream1: &str, stream2: &str) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] /Contents 6 0 R >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream1}\nendstream", stream1.len()),
        format!("<< /Length {} >>\nstream\n{stream2}\nendstream", stream2.len()),
    ];
    build_pdf(8, &bodies)
}

#[rstest]
#[case::zero_size(0, 10, ChunkConfigError::NonPositiveChunkSize { chunk_size: 0 })]
#[case::negative_size(-5, 10, ChunkConfigError::NonPositiveChunkSize { chunk_size: -5 })]
#[case::negative_overlap(100, -1, ChunkConfigError::NegativeChunkOverlap { chunk_overlap: -1 })]
#[case::overlap_equals_size(100, 100, ChunkConfigError::OverlapNotLessThanSize { chunk_size: 100, chunk_overlap: 100 })]
#[case::overlap_exceeds_size(100, 150, ChunkConfigError::OverlapNotLessThanSize { chunk_size: 100, chunk_overlap: 150 })]
fn chunk_config_rejects_invalid_values(
    #[case] chunk_size: i64,
    #[case] chunk_overlap: i64,
    #[case] expected: ChunkConfigError,
) {
    let error = RecursiveCharacterTextSplitter::new(chunk_size, chunk_overlap, None)
        .expect_err("the configuration is invalid");
    assert_eq!(error, expected);
}

#[test]
fn chunk_config_accepts_a_valid_configuration() {
    RecursiveCharacterTextSplitter::new(100, 20, None).expect("the configuration is valid");
}

#[test]
fn chunk_defaults_are_hardcoded_constants() {
    assert_eq!(DEFAULT_CHUNK_SIZE, 1000);
    assert_eq!(DEFAULT_CHUNK_OVERLAP, 200);
    assert_eq!(DEFAULT_MAX_RECURSE_DEPTH, 100);
}

#[test]
fn splitter_basic_splits_on_the_default_separators() {
    let splitter =
        RecursiveCharacterTextSplitter::new(10, 0, None).expect("the configuration is valid");
    let chunks = splitter.split_text("one two three four five six seven");
    assert_eq!(
        chunks,
        vec![
            "one two".to_owned(),
            "two three".to_owned(),
            "three four".to_owned(),
            "four five".to_owned(),
            "five six".to_owned(),
            "six seven".to_owned(),
        ]
    );
}

#[test]
fn splitter_overlap_repeats_text_between_adjacent_chunks() {
    let splitter =
        RecursiveCharacterTextSplitter::new(20, 5, Some(vec![" ".to_owned(), String::new()]))
            .expect("the configuration is valid");
    let chunks = splitter.split_text("alpha beta gamma delta epsilon zeta eta theta");
    assert!(chunks.len() > 1);
}

#[test]
fn splitter_separators_prefers_the_earliest_separator_present_in_the_text() {
    let splitter = RecursiveCharacterTextSplitter::new(5, 0, None).expect("valid configuration");
    let chunks = splitter.split_text("a\n\nb\nc");
    assert_eq!(chunks, vec!["a".to_owned(), "a\n\nb\nc".to_owned()]);
}

#[test]
fn splitter_recursion_falls_back_to_the_narrower_separator() {
    let splitter = RecursiveCharacterTextSplitter::new(
        3,
        0,
        Some(vec!["\n\n".to_owned(), " ".to_owned(), String::new()]),
    )
    .expect("valid configuration");
    let chunks = splitter.split_text("ab cd\n\nef gh");
    assert_eq!(
        chunks,
        vec![
            "ab".to_owned(),
            "ab cd".to_owned(),
            "ef".to_owned(),
            "ef gh".to_owned(),
        ]
    );
}

#[test]
fn splitter_force_split_strides_with_overlap_when_no_separator_matches() {
    let splitter =
        RecursiveCharacterTextSplitter::new(4, 1, Some(vec!["Q".to_owned(), "Z".to_owned()]))
            .expect("valid configuration");
    let chunks = splitter.split_text("abcdefgh");
    assert_eq!(
        chunks,
        vec!["abcd".to_owned(), "defg".to_owned(), "gh".to_owned()]
    );
}

#[test]
fn splitter_force_split_stops_exactly_at_the_end_of_the_text() {
    let splitter =
        RecursiveCharacterTextSplitter::new(4, 1, Some(vec!["Q".to_owned(), "Z".to_owned()]))
            .expect("valid configuration");
    let chunks = splitter.split_text("abcdefg");
    assert_eq!(chunks, vec!["abcd".to_owned(), "defg".to_owned()]);
}

#[test]
fn assemble_chunks_sends_a_split_at_the_chunk_size_boundary_to_force_split() {
    let splitter = RecursiveCharacterTextSplitter::new(4, 0, Some(vec!["|".to_owned()]))
        .expect("valid configuration");
    let chunks = splitter.split_text("abcd|efgh");
    assert_eq!(chunks, vec!["abcd".to_owned(), "efgh".to_owned()]);
}

#[test]
fn assemble_chunks_flushes_pending_good_splits_before_force_splitting_an_oversized_one() {
    let splitter = RecursiveCharacterTextSplitter::new(5, 0, Some(vec!["|".to_owned()]))
        .expect("valid configuration");
    let chunks = splitter.split_text("ab|verylongpiece");
    assert_eq!(
        chunks,
        vec![
            "ab".to_owned(),
            "veryl".to_owned(),
            "ongpi".to_owned(),
            "ece".to_owned(),
        ]
    );
}

#[test]
fn splitter_force_split_trims_python_whitespace_control_characters() {
    let splitter = RecursiveCharacterTextSplitter::new(3, 0, Some(vec!["Q".to_owned()]))
        .expect("valid configuration");
    let chunks = splitter.split_text("\u{1c}doc\u{1f}");
    assert_eq!(chunks, vec!["do".to_owned(), "c".to_owned()]);
}

#[test]
fn splitter_merge_splits_trims_python_whitespace_control_characters() {
    let splitter = RecursiveCharacterTextSplitter::new(10, 0, Some(vec![String::new()]))
        .expect("valid configuration");
    let chunks = splitter.split_text("\u{1c}doc\u{1f}");
    assert_eq!(chunks, vec!["doc".to_owned()]);
}

#[test]
fn splitter_recursion_depth_bound_separates_normal_splitting_from_character_fallback() {
    let splitter =
        RecursiveCharacterTextSplitter::new(4, 2, Some(vec![String::new()])).expect("valid");
    let separators = vec![String::new()];

    let at_limit = splitter.split_recursive("abcdefgh", &separators, DEFAULT_MAX_RECURSE_DEPTH);
    assert_eq!(
        at_limit,
        vec!["abcd".to_owned(), "cdef".to_owned(), "efgh".to_owned()]
    );

    let past_limit = splitter.split_recursive(
        "abcdefgh",
        &separators,
        DEFAULT_MAX_RECURSE_DEPTH.saturating_add(1),
    );
    assert_eq!(past_limit, vec!["abcd".to_owned(), "efgh".to_owned()]);
}

#[test]
fn splitter_counts_unicode_characters_not_bytes() {
    let splitter =
        RecursiveCharacterTextSplitter::new(4, 0, Some(vec![String::new()])).expect("valid");
    let chunks = splitter.split_text("\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30b9}\u{30c8}");
    assert_eq!(
        chunks,
        vec![
            "\u{65e5}\u{672c}\u{8a9e}\u{306e}".to_owned(),
            "\u{306e}\u{30c6}\u{30b9}\u{30c8}".to_owned(),
        ]
    );
}

#[test]
fn pdf_extracts_text_from_a_well_formed_document() {
    let pdf = minimal_pdf("Test PDF File");
    let extracted = extract_text_from_pdf(&pdf);
    assert_eq!(extracted, Some("Test PDF File".to_owned()));
}

#[test]
fn pdf_returns_none_for_a_page_with_no_text_showing_operators() {
    let pdf = pdf_with_stream("BT ET");
    assert_eq!(extract_text_from_pdf(&pdf), None);
}

#[test]
fn pdf_returns_none_when_any_page_fails_to_extract() {
    let pdf = two_page_pdf("BT /F1 12 Tf 100 700 Td (Test PDF File) Tj ET", "BT Tf ET");
    assert_eq!(extract_text_from_pdf(&pdf), None);
}

#[test]
fn pdf_returns_none_for_non_pdf_bytes() {
    assert_eq!(extract_text_from_pdf(b"this is not a pdf"), None);
}

#[test]
fn pdf_returns_none_without_panicking_on_truncated_bytes() {
    let pdf = minimal_pdf("Test PDF File");
    let truncated = pdf
        .get(..pdf.len().checked_div(2).expect("2 is nonzero"))
        .expect("length is within bounds");
    let _extracted = extract_text_from_pdf(truncated);
}

#[test]
fn pdf_returns_none_for_empty_input() {
    assert_eq!(extract_text_from_pdf(&[]), None);
}

#[test]
fn pdf_extracts_a_page_within_the_decompressed_size_limit() {
    let large_text = "A".repeat(5_000_000);
    let pdf = minimal_pdf(&large_text);
    assert_eq!(extract_text_from_pdf(&pdf), Some(large_text));
}

#[rstest]
#[case::openai("openai")]
#[case::bedrock("bedrock")]
#[case::gemini("gemini")]
#[case::s3_vectors("s3_vectors")]
#[case::vertex_ai("vertex_ai")]
fn get_ingestion_class_resolves_every_supported_provider(#[case] provider: &str) {
    get_ingestion_class(provider).expect("provider is supported");
}

#[test]
fn get_ingestion_class_rejects_an_unsupported_provider() {
    let error = get_ingestion_class("unknown-provider").expect_err("provider is not supported");
    let IngestionProviderError::Unsupported {
        provider,
        supported,
    } = error;
    assert_eq!(provider, "unknown-provider");
    assert_eq!(supported, "openai, bedrock, gemini, s3_vectors, vertex_ai");
}

#[rstest]
fn replays_the_pdf_extraction_corpus(#[files("fuzz/corpus/pdf_extraction/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus seed file reads");
    let _extracted = extract_text_from_pdf(&bytes);
}
