#![cfg(test)]

use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
use aws_smithy_types::body::SdkBody;
use http::{Request, Response};

pub(super) fn replay(body: &str, status: u16) -> StaticReplayClient {
    StaticReplayClient::new(vec![ReplayEvent::new(
        Request::builder()
            .method("POST")
            .uri("https://kms.us-east-1.amazonaws.com/")
            .body(SdkBody::from(r#"{"CiphertextBlob":"aGVsbG8="}"#))
            .unwrap(),
        Response::builder()
            .status(status)
            .body(SdkBody::from(body))
            .unwrap(),
    )])
}
