#![cfg(test)]

use super::edit;
use crate::completion::setup::outbound_client;
use crate::images::convert::parse;
use crate::images::session::ImageSession;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
async fn edit_sends_multipart_request_and_decodes_response() {
    let serving = serve(
        reply("200 OK", &[], r#"{"created":7,"data":[]}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut blobs = vec![vec![1_u8, 2_u8]];
    let payload = format!(
        r#"{{"arguments":{{"prompt":"cat","api_key":"sk-123456789012345678901234567890","api_base":"http://{}/v1","image":{{"$file":{{"name":"cat.png","blob":0}}}}}}}}"#,
        serving.address
    );
    let (call, files) = parse(&payload, &mut blobs).unwrap();
    let environment = BTreeMap::new();
    let session = ImageSession {
        client: outbound_client(&environment).unwrap(),
        environment,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let answer = edit(&session, call, files).await.unwrap();
    assert_eq!(answer.reply.created, 7);
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/images/edits HTTP/1.1");
    assert!(sent.head.contains("content-type: multipart/form-data"));
    assert!(sent.body.contains("filename=\"cat.png\""));
    assert!(sent.body.contains("\r\n\r\n\u{1}\u{2}\r\n"));
}
