#![cfg(test)]

use crate::error::HttpError;
use crate::url_path::{encode_segment, encode_segments};

#[test]
fn url_path_tests_behavior() {
    assert_eq!(
        encode_segment("a/b?c#d", "path parameter").unwrap(),
        "a%2Fb%3Fc%23d"
    );
    assert_eq!(encode_segments("a/b?c#d", "path").unwrap(), "a/b%3Fc%23d");
    assert_eq!(encode_segment("x\u{e9}", "id").unwrap(), "x%C3%A9");
    assert_eq!(
        encode_segment("-._~ !*'()", "id").unwrap(),
        "-._~%20%21%2A%27%28%29"
    );
    assert_eq!(
        encode_segments("\u{3b1}/\u{3b2}", "path").unwrap(),
        "%CE%B1/%CE%B2"
    );
}

#[rstest::rstest]
#[case::single_empty(encode_segment("", "file_id"), "file_id is required")]
#[case::multi_empty(encode_segments("", "path"), "path is required")]
#[case::leading_empty(encode_segments("/value", "path"), "path is required")]
#[case::trailing_empty(encode_segments("value/", "path"), "path is required")]
#[case::middle_empty(encode_segments("value//other", "path"), "path is required")]
#[case::single_dot(encode_segment(".", "file_id"), "file_id cannot be a dot path segment")]
#[case::single_dotdot(
    encode_segment("..", "file_id"),
    "file_id cannot be a dot path segment"
)]
#[case::multi_dot(
    encode_segments("value/..", "path"),
    "path cannot be a dot path segment"
)]
fn rejects_invalid_segments(#[case] result: Result<String, HttpError>, #[case] expected: &str) {
    assert_eq!(result.unwrap_err().to_string(), expected);
}
