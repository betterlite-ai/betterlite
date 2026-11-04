#![cfg(test)]

use rstest::rstest;

use super::{MAX_EXTRACTED_PDF_TEXT_BYTES, append_page};

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
