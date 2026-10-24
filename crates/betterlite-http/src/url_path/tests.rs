#![cfg(test)]

use crate::error::UrlPathError;
use crate::url_path::{encode_url_path_segment, encode_url_path_segments};

#[test]
fn b171_helper_runner_tests_behavior() {
    assert_eq!(
        encode_url_path_segment("a/b?c#d"),
        Ok("a%2Fb%3Fc%23d".to_owned())
    );
    assert_eq!(
        encode_url_path_segments("a/b?c#d"),
        Ok("a/b%3Fc%23d".to_owned())
    );
    assert_eq!(encode_url_path_segment("x\u{e9}"), Ok("x%C3%A9".to_owned()));
    assert_eq!(
        encode_url_path_segments("\u{3b1}/\u{3b2}"),
        Ok("%CE%B1/%CE%B2".to_owned())
    );
}

#[rstest::rstest]
#[case::single_empty(encode_url_path_segment(""), UrlPathError::Required { field_name: "path parameter".to_owned() })]
#[case::multi_empty(encode_url_path_segments(""), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::leading_empty(encode_url_path_segments("/value"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::trailing_empty(encode_url_path_segments("value/"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::middle_empty(encode_url_path_segments("value//other"), UrlPathError::Required { field_name: "path".to_owned() })]
#[case::single_dot(encode_url_path_segment("."), UrlPathError::DotSegment { field_name: "path parameter".to_owned() })]
#[case::single_dotdot(encode_url_path_segment(".."), UrlPathError::DotSegment { field_name: "path parameter".to_owned() })]
#[case::multi_dot(encode_url_path_segments("value/.."), UrlPathError::DotSegment { field_name: "path".to_owned() })]
fn rejects_invalid_segments(
    #[case] result: Result<String, UrlPathError>,
    #[case] expected: UrlPathError,
) {
    assert_eq!(result, Err(expected));
}
