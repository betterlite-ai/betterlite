#![cfg(test)]

use super::variation;
use crate::completion::setup::outbound_client;
use crate::images::convert::parse;
use crate::images::session::ImageSession;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
async fn variation_sends_file_and_decodes_response() {
    let serving = serve(
        reply("200 OK", &[], r#"{"created":9,"data":[]}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut blobs = vec![vec![3_u8, 4_u8]];
    let payload = format!(
        r#"{{"arguments":{{"model":"dall-e-2","api_key":"sk-123456789012345678901234567890","api_base":"http://{}/v1","image":{{"$file":{{"name":"cat.png","blob":0}}}}}}}}"#,
        serving.address
    );
    let (call, files) = parse(&payload, &mut blobs).unwrap();
    let environment = BTreeMap::new();
    let session = ImageSession {
        client: outbound_client(&environment).unwrap(),
        environment,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let answer = variation(&session, call, files, false).await.unwrap();
    assert_eq!(answer.reply.created, 9);
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/images/variations HTTP/1.1");
    assert!(sent.head.contains("content-type: multipart/form-data"));
    assert!(sent.body.contains("filename=\"cat.png\""));
}

#[rstest]
#[case::unauthorized("401 Unauthorized", "AuthenticationError")]
#[case::server_error("500 Internal Server Error", "InternalServerError")]
#[tokio::test]
async fn variation_maps_provider_status_and_message(#[case] status: &str, #[case] kind: &str) {
    let serving = serve(
        reply(status, &[], r#"{"error":{"message":"bad key"}}"#),
        Duration::ZERO,
    )
    .unwrap();
    let mut blobs = vec![vec![3_u8]];
    let payload = format!(
        r#"{{"arguments":{{"model":"dall-e-2","api_key":"sk-123456789012345678901234567890","api_base":"http://{}/v1","image":{{"$file":{{"name":"cat.png","blob":0}}}}}}}}"#,
        serving.address
    );
    let (call, files) = parse(&payload, &mut blobs).unwrap();
    let environment = BTreeMap::new();
    let session = ImageSession {
        client: outbound_client(&environment).unwrap(),
        environment,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    let failure = variation(&session, call, files, false).await.unwrap_err();
    assert!(failure.message.contains("bad key"));
    assert_eq!(failure.kind, kind);
    assert_eq!(failure.model.as_deref(), Some("dall-e-2"));
    let _request = serving.finish().unwrap();
}
