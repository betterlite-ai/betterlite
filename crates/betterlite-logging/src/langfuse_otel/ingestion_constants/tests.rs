#![cfg(test)]

use rstest::rstest;

use super::{LANGFUSE_INGESTION_VERSION, LANGFUSE_VERSION_HEADER};

#[rstest]
#[case::header_name(LANGFUSE_VERSION_HEADER, "x-langfuse-ingestion-version")]
#[case::version_is_text(LANGFUSE_INGESTION_VERSION, "4")]
fn ingestion_constants_match_langfuse_v4(#[case] constant: &str, #[case] expected: &str) {
    assert_eq!(constant, expected);
}
