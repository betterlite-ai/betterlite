#![cfg(test)]

use crate::error::UrlPathError;
use crate::url_path::{encode_segment, encode_segments};

#[test]
fn b171_helper_runner_tests_behavior() {
    assert_eq!(
        encode_segment("a/b?c#d", "path parameter"),
        Ok("a%2Fb%3Fc%23d".to_owned())
    );
    assert_eq!(
        encode_segments("a/b?c#d", "path"),
        Ok("a/b%3Fc%23d".to_owned())
    );
    assert_eq!(encode_segment("x\u{e9}", "id"), Ok("x%C3%A9".to_owned()));
    assert_eq!(
        encode_segment("-._~ !*'()", "id"),
        Ok("-._~%20%21%2A%27%28%29".to_owned())
    );
    assert_eq!(
        encode_segments("\u{3b1}/\u{3b2}", "path"),
        Ok("%CE%B1/%CE%B2".to_owned())
    );
}

#[rstest::rstest]
#[case::single_empty(encode_segment("", "file_id"), UrlPathError::Required { field_name: "file_id".to_owned() })]
#[case::multi_empty(encode_segments("", "path"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::leading_empty(encode_segments("/value", "path"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::trailing_empty(encode_segments("value/", "path"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::middle_empty(encode_segments("value//other", "path"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::single_dot(encode_segment(".", "file_id"), UrlPathError::DotSegment { field_name: "file_id".to_owned() })]
#[case::single_dotdot(encode_segment("..", "file_id"), UrlPathError::DotSegment { field_name: "file_id".to_owned() })]
#[case::multi_dot(encode_segments("value/..", "path"), UrlPathError::DotSegment { field_name: "path".to_owned() })]
fn rejects_invalid_segments(
    #[case] result: Result<String, UrlPathError>,
    #[case] expected: UrlPathError,
) {
    assert_eq!(result, Err(expected));
}
