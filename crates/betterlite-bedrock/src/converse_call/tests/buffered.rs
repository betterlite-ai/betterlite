#![cfg(test)]

use super::{provider_error, timeout_error};
use crate::bedrock_settings::ResponseBody;
use crate::converse_call::error::ConverseCallError;
use crate::rerank::transport::RerankHttpReply;

#[rstest::rstest]
#[case::text(b"rate limited".to_vec(), "rate limited")]
#[case::non_utf8(vec![255, 254], "2 bytes that are not text")]
fn buffered_errors_preserve_status_headers_and_body(#[case] body: Vec<u8>, #[case] message: &str) {
    let reply = RerankHttpReply {
        status: 429,
        headers: vec![("x-request-id".to_owned(), "request-1".to_owned())],
        body,
    };
    let result = provider_error("https://example.test/converse", &reply).unwrap();
    let ConverseCallError::Provider { status, error } = result else {
        panic!("a provider error")
    };
    assert_eq!(status, 429);
    assert_eq!(error.error.message, message);
    assert_eq!(error.response_headers, reply.headers);
    assert_eq!(error.response_body, ResponseBody::Read(message.to_owned()));
}

#[test]
fn timeout_keeps_the_source_status_and_message() {
    let result = timeout_error().unwrap();
    assert_eq!(result.status(), Some(408));
    let ConverseCallError::Provider { error, .. } = result else {
        panic!("a provider error")
    };
    assert_eq!(error.error.message, "Timeout error occurred.");
}
