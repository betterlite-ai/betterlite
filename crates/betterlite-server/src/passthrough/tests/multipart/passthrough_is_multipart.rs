use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use rstest::rstest;

use crate::passthrough::multipart::is_multipart;

#[rstest]
#[case::exact(b"multipart/form-data", true)]
#[case::with_boundary(b"multipart/form-data; boundary=x", true)]
#[case::inside_another_value(b"text/plain; note=multipart/form-data", true)]
#[case::high_bytes(b"multipart/form-data\xff", true)]
#[case::json(b"application/json", false)]
#[case::urlencoded(b"application/x-www-form-urlencoded", false)]
#[case::empty(b"", false)]
fn looks_for_the_marker_inside_the_value(#[case] value: &[u8], #[case] expected: bool) {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_bytes(value).unwrap());
    assert_eq!(is_multipart(&headers), expected);
}

#[test]
fn a_request_without_a_content_type_is_not_multipart() {
    assert!(!is_multipart(&HeaderMap::new()));
}
