#![cfg(test)]

use rstest::rstest;

use super::read_request;
use crate::error::FixtureError;

#[rstest]
#[case::no_body("POST /v1beta/interactions HTTP/1.1\r\nhost: fixture\r\n\r\n", "")]
#[case::lowercase_length(
    "POST /v1beta/interactions HTTP/1.1\r\nhost: fixture\r\ncontent-length: 5\r\n\r\nhello",
    "hello"
)]
#[case::mixed_case_length(
    "POST /v1beta/interactions HTTP/1.1\r\nhost: fixture\r\nContent-Length: 5\r\n\r\nhello",
    "hello"
)]
fn reads_the_declared_body(#[case] raw: &str, #[case] expected: &str) {
    let mut stream = raw.as_bytes();

    let request = read_request(&mut stream).unwrap();

    assert_eq!(request.line, "POST /v1beta/interactions HTTP/1.1");
    assert_eq!(request.body, expected);
    assert!(request.head.ends_with("\r\n\r\n"));
    assert!(request.head.contains("host: fixture"));
}

#[test]
fn an_empty_stream_is_no_request() {
    let mut stream: &[u8] = b"";

    assert!(matches!(
        read_request(&mut stream),
        Err(FixtureError::NoRequest)
    ));
}
