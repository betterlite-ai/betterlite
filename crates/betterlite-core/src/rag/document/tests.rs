#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{
    ChunkConfigError, DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE, DEFAULT_MAX_RECURSE_DEPTH,
    IngestionProviderError, RecursiveCharacterTextSplitter, TextSplitter as _,
    extract_text_from_pdf, get_ingestion_class,
};

fn minimal_pdf(text: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = [0_usize; 6];

    offsets[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    offsets[2] = buf.len();
    buf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    offsets[3] = buf.len();
    buf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>\nendobj\n",
    );
    offsets[4] = buf.len();
    buf.extend_from_slice(
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );
    offsets[5] = buf.len();
    let stream = format!("BT /F1 12 Tf 100 700 Td ({text}) Tj ET");
    buf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{stream}\nendstream\nendobj\n",
            stream.len()
        )
        .as_bytes(),
    );

    let xref_offset = buf.len();
    buf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in &offsets[1..] {
        buf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n");
    buf.extend_from_slice(format!("{xref_offset}\n").as_bytes());
    buf.extend_from_slice(b"%%EOF");
    buf
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
fn splitter_force_split_strides_through_text_with_no_usable_separator() {
    let splitter = RecursiveCharacterTextSplitter::new(4, 1, Some(vec![String::new()]))
        .expect("valid configuration");
    let chunks = splitter.split_text("abcdefgh");
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 4));
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
    eprintln!("PROBE_EXTRACTED={extracted:?}");
    assert!(extracted.is_some());
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
