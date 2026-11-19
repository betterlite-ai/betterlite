use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::azure_stream_errors::content_filter::decode_content_filter_error;

#[rstest]
fn replays_the_azure_stream_errors_corpus(
    #[files("fuzz/corpus/azure_stream_errors/*")] path: PathBuf,
) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let bytes = fs::read(&path).expect("the seed is readable");
    let decoded = decode_content_filter_error(&bytes);
    if name.starts_with("malformed") || name.starts_with("truncated") {
        assert!(matches!(decoded, Err(_)), "{name} must fail to parse");
    } else if name.starts_with("valid") {
        assert!(
            decoded.expect("a valid seed parses").is_some(),
            "{name} must be a content-filter error"
        );
    } else if name.starts_with("boundary") {
        decoded.expect("a boundary seed parses");
    }
}

#[test]
fn a_plain_error_body_is_not_a_content_filter() {
    let decoded = decode_content_filter_error(
        br#"{"error":{"code":"invalid_request_error","type":"invalid_request_error","message":"no filter here"}}"#,
    )
    .expect("a plain error body parses");
    assert!(decoded.is_none(), "no filter signal means no filter error");
}
