#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::rag::document::IngestionProvider;
use rstest::rstest;

use super::pdf::extract_text_from_pdf;
use super::provider::{IngestionProviderError, get_ingestion_class};
use super::splitter::{
    ChunkConfigError, DEFAULT_MAX_RECURSE_DEPTH, RecursiveCharacterTextSplitter, TextSplitter as _,
};

fn default_separators() -> Vec<String> {
    ["\n\n", "\n", " ", ""]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

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
    let error = RecursiveCharacterTextSplitter::new(chunk_size, chunk_overlap, Vec::new())
        .expect_err("the configuration is invalid");
    assert_eq!(error, expected);
}

#[test]
fn chunk_config_accepts_a_valid_configuration() {
    RecursiveCharacterTextSplitter::new(100, 20, Vec::new()).expect("the configuration is valid");
}

#[test]
fn chunk_defaults_are_hardcoded_constants() {
    assert_eq!(DEFAULT_MAX_RECURSE_DEPTH, 100);
}

#[test]
fn splitter_basic_splits_on_the_default_separators() {
    let splitter = RecursiveCharacterTextSplitter::new(10, 0, default_separators())
        .expect("the configuration is valid");
    let chunks = splitter
        .split_text("one two three four five six seven")
        .expect("under cap");
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
    let splitter = RecursiveCharacterTextSplitter::new(20, 5, vec![" ".to_owned(), String::new()])
        .expect("the configuration is valid");
    let chunks = splitter
        .split_text("alpha beta gamma delta epsilon zeta eta theta")
        .expect("under cap");
    assert_eq!(
        chunks,
        vec![
            "alpha beta gamma".to_owned(),
            "gamma delta epsilon".to_owned(),
            "epsilon zeta eta".to_owned(),
            "eta theta".to_owned(),
        ]
    );
}

#[test]
fn splitter_separators_prefers_the_earliest_separator_present_in_the_text() {
    let splitter = RecursiveCharacterTextSplitter::new(5, 0, default_separators())
        .expect("valid configuration");
    let chunks = splitter.split_text("a\n\nb\nc").expect("under cap");
    assert_eq!(chunks, vec!["a".to_owned(), "a\n\nb\nc".to_owned()]);
}

#[test]
fn splitter_recursion_falls_back_to_the_narrower_separator() {
    let splitter = RecursiveCharacterTextSplitter::new(
        3,
        0,
        vec!["\n\n".to_owned(), " ".to_owned(), String::new()],
    )
    .expect("valid configuration");
    let chunks = splitter.split_text("ab cd\n\nef gh").expect("under cap");
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
    let splitter = RecursiveCharacterTextSplitter::new(4, 1, vec!["Q".to_owned(), "Z".to_owned()])
        .expect("valid configuration");
    let chunks = splitter.split_text("abcdefgh").expect("under cap");
    assert_eq!(
        chunks,
        vec!["abcd".to_owned(), "defg".to_owned(), "gh".to_owned()]
    );
}

#[test]
fn splitter_force_split_stops_exactly_at_the_end_of_the_text() {
    let splitter = RecursiveCharacterTextSplitter::new(4, 1, vec!["Q".to_owned(), "Z".to_owned()])
        .expect("valid configuration");
    let chunks = splitter.split_text("abcdefg").expect("under cap");
    assert_eq!(chunks, vec!["abcd".to_owned(), "defg".to_owned()]);
}

#[test]
fn chunk_output_cap_stops_the_force_split_path() {
    let splitter = RecursiveCharacterTextSplitter::new(1_000_000, 999_999, vec!["Q".to_owned()])
        .expect("valid overlap");
    let text = "a".repeat(1_000_500);
    let error = splitter.split_text(&text).expect_err("output exceeds cap");
    assert_eq!(
        error,
        super::splitter::output::SplitError::OutputTooLarge {
            limit: super::splitter::output::MAX_SPLIT_OUTPUT_BYTES,
        }
    );
    assert_eq!(
        error.to_string(),
        "text splitter output exceeds 268435456 bytes"
    );
}

#[test]
fn chunk_output_cap_stops_the_merge_path() {
    let splitter = RecursiveCharacterTextSplitter::new(1_000_000, 999_998, vec![" ".to_owned()])
        .expect("valid overlap");
    let text = "a ".repeat(600_000);
    assert_eq!(
        splitter.split_text(&text),
        Err(super::splitter::output::SplitError::OutputTooLarge {
            limit: super::splitter::output::MAX_SPLIT_OUTPUT_BYTES
        })
    );
}

#[test]
fn assemble_chunks_sends_a_split_at_the_chunk_size_boundary_to_force_split() {
    let splitter = RecursiveCharacterTextSplitter::new(4, 0, vec!["|".to_owned()])
        .expect("valid configuration");
    let chunks = splitter.split_text("abcd|efgh").expect("under cap");
    assert_eq!(chunks, vec!["abcd".to_owned(), "efgh".to_owned()]);
}

#[test]
fn assemble_chunks_flushes_pending_good_splits_before_force_splitting_an_oversized_one() {
    let splitter = RecursiveCharacterTextSplitter::new(5, 0, vec!["|".to_owned()])
        .expect("valid configuration");
    let chunks = splitter.split_text("ab|verylongpiece").expect("under cap");
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
    let splitter = RecursiveCharacterTextSplitter::new(3, 0, vec!["Q".to_owned()])
        .expect("valid configuration");
    let chunks = splitter.split_text("\u{1c}doc\u{1f}").expect("under cap");
    assert_eq!(chunks, vec!["do".to_owned(), "c".to_owned()]);
}

#[test]
fn splitter_merge_splits_trims_python_whitespace_control_characters() {
    let splitter = RecursiveCharacterTextSplitter::new(10, 0, vec![String::new()])
        .expect("valid configuration");
    let chunks = splitter.split_text("\u{1c}doc\u{1f}").expect("under cap");
    assert_eq!(chunks, vec!["doc".to_owned()]);
}

#[test]
fn splitter_recursion_depth_bound_separates_normal_splitting_from_character_fallback() {
    let splitter = RecursiveCharacterTextSplitter::new(4, 2, vec![String::new()]).expect("valid");
    let separators = vec![String::new()];

    let mut at_limit = super::splitter::output::SplitOutput::default();
    splitter
        .split_recursive(
            "abcdefgh",
            &separators,
            DEFAULT_MAX_RECURSE_DEPTH,
            &mut at_limit,
        )
        .expect("under cap");
    assert_eq!(
        at_limit.chunks,
        vec!["abcd".to_owned(), "cdef".to_owned(), "efgh".to_owned()]
    );

    let mut past_limit = super::splitter::output::SplitOutput::default();
    splitter
        .split_recursive(
            "abcdefgh",
            &separators,
            DEFAULT_MAX_RECURSE_DEPTH.saturating_add(1),
            &mut past_limit,
        )
        .expect("under cap");
    assert_eq!(
        past_limit.chunks,
        vec!["abcd".to_owned(), "efgh".to_owned()]
    );
}

#[test]
fn splitter_counts_unicode_characters_not_bytes() {
    let splitter = RecursiveCharacterTextSplitter::new(4, 0, vec![String::new()]).expect("valid");
    let chunks = splitter
        .split_text("\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30b9}\u{30c8}")
        .expect("under cap");
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
fn pdf_line_move_td_starts_a_new_line() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -14 Td (World) Tj ET"
        ))
        .as_deref(),
        Some("Hello\nWorld")
    );
}

#[test]
fn pdf_line_move_td_leading_starts_a_new_line() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -14 TD (World) Tj T* (Again) Tj ET"
        ))
        .as_deref(),
        Some("Hello\nWorld\nAgain")
    );
}

#[test]
fn pdf_line_move_tm_starts_a_new_line() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 12 Tf 1 0 0 1 100 700 Tm (Hello) Tj 1 0 0 1 100 680 Tm (World) Tj ET"
        ))
        .as_deref(),
        Some("Hello\nWorld")
    );
}

#[test]
fn pdf_line_move_bare_t_star_does_not_start_a_line() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 12 Tf 100 700 Td (Hello) Tj T* (World) Tj ET"
        ))
        .as_deref(),
        Some("HelloWorld")
    );
}

#[test]
fn pdf_line_move_scaled_tm_uses_font_scale() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 1 Tf 12 0 0 12 100 700 Tm (Hello) Tj 12 0 0 12 100 695 Tm (World) Tj ET"
        ))
        .as_deref(),
        Some("HelloWorld")
    );
}

#[test]
fn pdf_line_move_tj_uses_vertical_baseline() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 12 Tf 100 700 Td (Hello) Tj 1 0 0 1 0 -20 cm (World) Tj ET"
        ))
        .as_deref(),
        Some("HelloWorld\n")
    );
}

#[test]
fn pdf_line_move_t_star_uses_text_leading() {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(
            "BT /F1 12 Tf 14 TL 100 700 Td (Hello) Tj T* (World) Tj ET"
        ))
        .as_deref(),
        Some("Hello\nWorld")
    );
}

#[rstest]
#[case::second_bt(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj ET BT 100 700 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::empty_moves("BT /F1 12 Tf 100 700 Td 0 -14 Td (Hello) Tj ET", "Hello")]
#[case::repeated_moves(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -14 Td 0 -14 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::scaled_td(
    "BT /F1 1 Tf 12 0 0 12 100 700 Tm (Hello) Tj 0 -1 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::scaled_t_star(
    "BT /F1 1 Tf 1 TL 12 0 0 12 100 700 Tm (Hello) Tj T* (World) Tj ET",
    "Hello\nWorld"
)]
#[case::changed_scale(
    "BT /F1 1 Tf 12 0 0 12 100 700 Tm (Hello) Tj 24 0 0 24 100 685 Tm (World) Tj ET",
    "Hello\nWorld"
)]
#[case::graphics_scale(
    "2 0 0 2 0 0 cm BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -5 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::graphics_translation(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 1 0 0 1 0 -14 cm 0 0 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::graphics_restore(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj q 1 0 0 1 0 -14 cm Q 0 0 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::rotated_tm(
    "BT /F1 12 Tf 0 1 -1 0 100 700 Tm (Hello) Tj 0 -14 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::quote_leading(
    "BT /F1 12 Tf 14 TL 100 700 Td (Hello) Tj (World) ' ET",
    "Hello\nWorld"
)]
#[case::double_quote(
    "BT /F1 12 Tf 14 TL 100 700 Td (Hello) Tj 0 0 (World) \" ET",
    "Hello\nWorld"
)]
#[case::skewed_tm(
    "BT /F1 12 Tf 0 1 -1 1 100 700 Tm (Hello) Tj 20 0 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::orientation_changes_axis(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 1 -1 0 100 700 Tm (World) Tj ET",
    "HelloWorld"
)]
#[case::orientation_resets_span(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 1 -1 0 109 700 Tm (World) Tj 0 1 -1 0 99 700 Tm (Again) Tj ET",
    "HelloWorld\nAgain"
)]
#[case::newline_resets_span(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -20 Td 0 -5 Td (World) Tj 0 10 Td (Again) Tj ET",
    "Hello\nWorld\nAgain"
)]
#[case::empty_text_resets_span(
    "BT /F1 12 Tf 100 700 Td () Tj 0 -5 Td (Hello) Tj 0 10 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::threshold_boundary(
    "BT /F1 5 Tf 100 700 Td (Hello) Tj 0 -4 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::line_span(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -5 Td (World) Tj 0 10 Td (Again) Tj ET",
    "HelloWorldAgain"
)]
#[case::td_missing("BT /F1 12 Tf 100 700 Td (Hello) Tj Td (World) Tj ET", "HelloWorld")]
#[case::td_missing_y("BT /F1 12 Tf 100 700 Td (Hello) Tj 0 Td (World) Tj ET", "HelloWorld")]
#[case::td_bad_x(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj /bad -14 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::td_bad_y(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 /bad Td (World) Tj ET",
    "HelloWorld"
)]
#[case::td_out_of_range(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 2147483648 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::td_lower_out_of_range(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 0 -2147483649 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::td_leading_missing(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 14 TL TD T* (World) Tj ET",
    "Hello\nWorld"
)]
#[case::td_leading_bad_x(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj /bad -14 TD T* (World) Tj ET",
    "Hello\nWorld"
)]
#[case::td_leading_bad_y(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 14 TL 0 /bad TD T* (World) Tj ET",
    "Hello\nWorld"
)]
#[case::tl_missing(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 14 TL TL T* (World) Tj ET",
    "HelloWorld"
)]
#[case::tl_bad(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 14 TL /bad TL T* (World) Tj ET",
    "HelloWorld"
)]
#[case::tm_short(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 1 0 Tm (World) Tj ET",
    "Hello\nWorld"
)]
#[case::tm_bad(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 1 0 /bad 1 100 700 Tm (World) Tj ET",
    "Hello\nWorld"
)]
#[case::cm_short(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 1 0 cm 0 0 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::cm_bad(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 1 0 /bad 1 0 14 cm 0 0 Td (World) Tj ET",
    "HelloWorld"
)]
#[case::tj_missing("BT /F1 12 Tf 100 700 Td (Hello) Tj Tj (World) Tj ET", "HelloWorld")]
#[case::tj_non_string("BT /F1 12 Tf 100 700 Td (Hello) Tj 42 Tj (World) Tj ET", "HelloWorld")]
#[case::quote_short(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj 14 TL 0 0 \" (World) Tj ET",
    "HelloWorld"
)]
#[case::cm_resets_identity(
    "1 0 0 1 0 100 cm BT /F1 12 Tf 100 700 Td (Hello) Tj cm 0 0 Td (World) Tj ET",
    "Hello\nWorld"
)]
#[case::tj_array_missing("BT /F1 12 Tf 100 700 Td (Hello) Tj TJ (World) Tj ET", "HelloWorld")]
#[case::tj_array_non_array("BT /F1 12 Tf 100 700 Td (Hello) Tj 42 TJ (World) Tj ET", "HelloWorld")]
#[case::tf_bad_size_keeps_previous(
    "BT /F1 12 Tf 100 700 Td (Hello) Tj /F1 /bad Tf 0 -5 Td (World) Tj ET",
    "HelloWorld"
)]
fn pdf_line_move_tracks_text_state(#[case] stream: &str, #[case] expected: &str) {
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(stream)).as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::below_threshold("-101", "Hello World ")]
#[case::at_threshold("-100", "HelloWorld ")]
#[case::above_threshold("-99", "HelloWorld ")]
#[case::zero("0", "HelloWorld ")]
#[case::positive("99", "HelloWorld ")]
#[case::real_offset("-101.5", "HelloWorld ")]
#[case::non_numeric_offsets("/bad true null [] <<>>", "HelloWorld ")]
fn pdf_retains_lopdf_tj_spacing(#[case] offset: &str, #[case] expected: &str) {
    let stream = format!("BT /F1 12 Tf [(Hello) {offset} (World)] TJ ET");
    assert_eq!(
        extract_text_from_pdf(&pdf_with_stream(&stream)).as_deref(),
        Some(expected)
    );
}

#[test]
fn pdf_returns_none_for_a_page_with_no_text_showing_operators() {
    let pdf = pdf_with_stream("BT ET");
    assert_eq!(extract_text_from_pdf(&pdf), None);
}

#[rstest]
#[case::two_text_pages(
    "BT /F1 12 Tf (First) Tj ET",
    "BT /F1 12 Tf (Second) Tj ET",
    "First\n\nSecond"
)]
#[case::blank_first("BT ET", "BT /F1 12 Tf (Second) Tj ET", "Second")]
#[case::blank_last("BT /F1 12 Tf (First) Tj ET", "BT ET", "First")]
fn pdf_joins_nonempty_pages(#[case] first: &str, #[case] second: &str, #[case] expected: &str) {
    assert_eq!(
        extract_text_from_pdf(&two_page_pdf(first, second)).as_deref(),
        Some(expected)
    );
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
    assert_eq!(extract_text_from_pdf(truncated), None);
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

#[test]
fn pdf_returns_none_past_the_decompressed_page_limit() {
    let spaces = " ".repeat(65 * 1024 * 1024);
    let pdf = pdf_with_stream(&format!("BT /F1 12 Tf {spaces}(Hello) Tj ET"));
    let mut document = lopdf::Document::load_mem(&pdf).expect("valid PDF");
    document
        .get_object_mut((5, 0))
        .expect("content object")
        .as_stream_mut()
        .expect("stream")
        .compress()
        .expect("compress");
    let mut compressed = Vec::new();
    document.save_to(&mut compressed).expect("save PDF");
    assert_eq!(extract_text_from_pdf(&compressed), None);
}

#[test]
fn pdf_returns_none_past_the_total_text_cap() {
    let text = "A".repeat(33 * 1024 * 1024);
    assert_eq!(
        extract_text_from_pdf(&minimal_pdf(&text)).as_deref(),
        Some(text.as_str())
    );
    let stream = format!("BT /F1 12 Tf ({text}) Tj ET");
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>"
            .to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>"
            .to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ),
    ];
    let pdf = build_pdf(7, &bodies);
    assert_eq!(extract_text_from_pdf(&pdf), None);
}

#[rstest]
#[case::openai("openai", IngestionProvider::Openai)]
#[case::bedrock("bedrock", IngestionProvider::Bedrock)]
#[case::gemini("gemini", IngestionProvider::Gemini)]
#[case::s3_vectors("s3_vectors", IngestionProvider::S3Vectors)]
#[case::vertex_ai("vertex_ai", IngestionProvider::VertexAi)]
fn get_ingestion_class_maps_each_name_to_its_variant(
    #[case] provider: &str,
    #[case] expected: IngestionProvider,
) {
    assert_eq!(get_ingestion_class(provider), Ok(expected));
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

#[test]
fn pdf_skips_an_absent_font_and_keeps_known_text() {
    let pdf = pdf_with_stream("BT /Missing 12 Tf (Unknown) Tj /F1 12 Tf (Known) Tj ET");
    assert_eq!(extract_text_from_pdf(&pdf).as_deref(), Some("Known"));
}
