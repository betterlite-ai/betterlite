#![cfg(test)]

use lopdf::{Document, Stream, dictionary};
use rstest::rstest;

use super::{MAX_EXTRACTED_PDF_TEXT_BYTES, append_page, extract_page_text};

#[rstest]
#[case::exact_limit(3, true)]
#[case::separator_crosses_limit(2, false)]
fn joined_text_cap_counts_page_separators(#[case] remaining: usize, #[case] allowed: bool) {
    let prefix_len = MAX_EXTRACTED_PDF_TEXT_BYTES
        .checked_sub(remaining)
        .expect("room for the suffix");
    let mut output = "A".repeat(prefix_len);
    let original_length = output.len();
    assert_eq!(append_page(&mut output, "B").is_some(), allowed);
    if allowed {
        assert_eq!(output.len(), MAX_EXTRACTED_PDF_TEXT_BYTES);
        assert!(output.ends_with("\n\nB"));
    } else {
        assert_eq!(output.len(), original_length);
    }
}

#[rstest]
#[case::exact_limit(0, Some(MAX_EXTRACTED_PDF_TEXT_BYTES))]
#[case::past_limit(1, None)]
fn extracted_page_cap_counts_decoded_bytes(#[case] extra: usize, #[case] expected: Option<usize>) {
    let mut content = b"BT /F1 12 Tf (".to_vec();
    // Each WinAnsi é decodes into two UTF-8 bytes, filling the 64 MiB output cap.
    content.extend(vec![0xe9; 0x0200_0000]);
    content.extend(vec![b'A'; extra]);
    content.extend_from_slice(b") Tj ET");
    let mut document = Document::new();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let contents = document.add_object(Stream::new(dictionary! {}, content));
    let page = document.add_object(dictionary! {
        "Type" => "Page", "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        "Contents" => contents,
    });
    assert_eq!(
        extract_page_text(&document, page).map(|text| text.len()),
        expected
    );
}
