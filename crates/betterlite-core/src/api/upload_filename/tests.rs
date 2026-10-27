#![cfg(test)]

use super::{UploadFilenameError, safe_filename};
use rstest::rstest;

#[rstest]
#[case::unix("a/b.txt", "b.txt")]
#[case::windows(r"C:\temp\b.txt", "b.txt")]
#[case::parent("../b.txt", "b.txt")]
#[case::mixed("a\\b/c.txt", "c.txt")]
#[case::dotenv(".env", ".env")]
#[case::double_dot_extension("report..pdf", "report..pdf")]
#[case::spaces(" name ", " name ")]
fn basename_cases(#[case] filename: &str, #[case] expected: &str) {
    assert_eq!(safe_filename(filename), Ok(expected.to_owned()));
}

#[rstest]
#[case::empty("")]
#[case::trailing_slash("a/")]
#[case::dot(".")]
#[case::dot_dot("..")]
fn rejects_empty_or_unsafe(#[case] filename: &str) {
    assert_eq!(
        safe_filename(filename),
        Err(UploadFilenameError::EmptyOrUnsafe)
    );
}

#[rstest]
#[case::in_name("a\0b.txt")]
#[case::discarded_component("a\0/b.txt")]
fn rejects_null_bytes_before_extraction(#[case] filename: &str) {
    assert_eq!(safe_filename(filename), Err(UploadFilenameError::NullByte));
}
